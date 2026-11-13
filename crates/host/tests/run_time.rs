//! Parameters made while the plugin runs: a mixer whose host lists one gain,
//! with a private drive made once in `new`, and a list of groups the test
//! adds and removes, each with a volume and a mute that no host is shown.

#![cfg(all(feature = "vst3", feature = "cabi", feature = "test-host"))]

use std::cell::{Cell, RefCell};
use std::collections::BTreeSet;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, SyncSender, sync_channel};

use seam_host::{
    AuCodes, Block, Category, Driver, EventList, Layout, Maker, Parts, Plugin, PluginInfo,
    ProcessSetup, Processor, SavedState, Status,
};
use seam_learn::{Config, Control, DEFAULT_MAX_MAPPINGS, Mapping, Source, Takes};
use seam_params::{
    AnyParam, Bool, BoolDecl, Changes, Declarer, Float, FloatDecl, List, Origin, Params,
    audio_thread, host_id,
};
use seam_session::{ApplyError, Participant, Registration, Section};
use seam_ump::{ChannelVoice, Voice};
use seam_undo::{Bound, History, UndoSize};
use serde_json::{Value as Json, json};

seam_testkit::install_alloc_gate!();

const GAIN: FloatDecl = FloatDecl::new("gain", "Gain", -60.0..=12.0, 0.0)
    .with_step(0.1)
    .with_unit("dB")
    .checked();
const DRIVE: FloatDecl = FloatDecl::new("drive", "Drive", 0.0..=1.0, 0.0)
    .with_step(0.1)
    .checked();
const VOLUME: FloatDecl = FloatDecl::new("volume", "Volume", -60.0..=12.0, 0.0)
    .with_step(0.1)
    .with_unit("dB")
    .checked();
const MUTED: BoolDecl = BoolDecl::new("muted", "Muted", false).checked();

/// The one parameter the host lists.
#[derive(Debug, Clone)]
struct MixParams {
    gain: Float,
}

impl Params for MixParams {
    fn declare(p: &mut Declarer) -> Self {
        MixParams { gain: p.add(GAIN) }
    }
}

/// One group's parameters, made whenever a group is, declared as listed ones
/// are.
#[derive(Debug, Clone)]
struct GroupParams {
    volume: Float,
    muted: Bool,
}

impl Params for GroupParams {
    fn declare(p: &mut Declarer) -> Self {
        GroupParams {
            volume: p.add(VOLUME),
            muted: p.add(MUTED),
        }
    }
}

/// A node of the tree, as the main thread holds it. Its parameters are its
/// own; the list it sits in is what undo records adds and removes of.
#[derive(Debug)]
struct Group {
    id: u64,
    params: GroupParams,
}

impl UndoSize for Group {}

impl Group {
    /// Which group this is to the audio side: its stores, not its id. A
    /// load declares a group of the same id again, with new stores.
    fn store(&self) -> u64 {
        self.params.volume.erased().serial()
    }
}

fn factor(decibels: f32) -> f32 {
    10.0_f32.powf(decibels / 20.0)
}

/// A group's audio half: clones of its parameters, read as listed ones are,
/// with no lock and no tick between a write and the next block.
#[derive(Debug)]
struct GroupAudio {
    id: u64,
    params: GroupParams,
    changes: Changes,
    gain: f32,
}

impl GroupAudio {
    fn new(group: &Group) -> Self {
        GroupAudio {
            id: group.store(),
            params: group.params.clone(),
            changes: Changes::new(),
            gain: 1.0,
        }
    }

    fn retune(&mut self) -> f32 {
        let GroupParams { volume, muted } = &self.params;
        if self.changes.moved([volume, muted]) {
            self.gain = if muted.get() {
                0.0
            } else {
                factor(volume.get())
            };
        }
        self.gain
    }
}

/// How many groups the audio side holds without allocating.
const MOST_GROUPS: usize = 16;

/// What the main thread tells the audio side. A stand-in for the real
/// hand-off, a wait-free ring: two bounded channels.
#[derive(Debug)]
enum Order {
    Install(GroupAudio),
    Retire(u64),
}

/// The main thread's end of the hand-off.
#[derive(Debug)]
struct Bank {
    orders: SyncSender<Order>,
    retired: Receiver<GroupAudio>,
    installed: BTreeSet<u64>,
}

/// The audio thread's end.
#[derive(Debug)]
struct BankAudio {
    orders: Receiver<Order>,
    retired: SyncSender<GroupAudio>,
    groups: Vec<GroupAudio>,
}

fn bank() -> (Bank, BankAudio) {
    let (orders, from_main) = sync_channel(MOST_GROUPS * 2);
    let (to_main, retired) = sync_channel(MOST_GROUPS * 2);
    (
        Bank {
            orders,
            retired,
            installed: BTreeSet::new(),
        },
        BankAudio {
            orders: from_main,
            retired: to_main,
            groups: Vec::with_capacity(MOST_GROUPS),
        },
    )
}

impl Bank {
    /// Makes the audio side hold one half for each group in the list, and
    /// drops what it gave back. A tick's delay for structure: the plugin's
    /// idle runs it.
    fn follow(&mut self, groups: &List<Group>) {
        let wanted: Vec<(u64, Option<GroupAudio>)> = groups.with(|groups| {
            groups
                .iter()
                .map(|group| {
                    let new = !self.installed.contains(&group.store());
                    (group.store(), new.then(|| GroupAudio::new(group)))
                })
                .collect()
        });
        let keep: BTreeSet<u64> = wanted.iter().map(|(id, _)| *id).collect();
        for id in self
            .installed
            .difference(&keep)
            .copied()
            .collect::<Vec<_>>()
        {
            if self.orders.try_send(Order::Retire(id)).is_ok() {
                self.installed.remove(&id);
            }
        }
        for (id, half) in wanted {
            if let Some(half) = half
                && self.orders.try_send(Order::Install(half)).is_ok()
            {
                self.installed.insert(id);
            }
        }
        // Here, on the main thread, the retired halves and their
        // parameters drop.
        while self.retired.try_recv().is_ok() {}
    }
}

impl BankAudio {
    /// Takes the main thread's orders. Never allocates or frees: a retired
    /// half goes back to the main thread to drop.
    fn sync(&mut self) {
        while let Ok(order) = self.orders.try_recv() {
            match order {
                Order::Install(half) => {
                    if self.groups.len() < self.groups.capacity() {
                        self.groups.push(half);
                    } else {
                        let _ = self.retired.try_send(half);
                    }
                }
                Order::Retire(id) => {
                    if let Some(at) = self.groups.iter().position(|half| half.id == id) {
                        let half = self.groups.swap_remove(at);
                        let _ = self.retired.try_send(half);
                    }
                }
            }
        }
    }
}

/// The scope a group's parameters are saved and learned under.
fn scope(id: u64) -> String {
    format!("group/{id}")
}

/// How a test builds the mixer, on its own thread.
#[derive(Debug, Clone, Copy)]
struct Setup {
    /// The tree registers before learn is turned on, so a pass applies it
    /// first.
    tree_first: bool,
    /// The tree's apply keeps a group it holds already, instead of declaring
    /// it again.
    keep: bool,
    /// Learn's mapping ceiling.
    max_mappings: usize,
    /// The plugin lends its maker to [`MAKER`], for a test that declares
    /// from outside it through a test host, which gives no other way in.
    lend_maker: bool,
}

impl Default for Setup {
    fn default() -> Self {
        Setup {
            tree_first: false,
            keep: false,
            max_mappings: DEFAULT_MAX_MAPPINGS,
            lend_maker: false,
        }
    }
}

thread_local! {
    static SETUP: Cell<Setup> = Cell::new(Setup::default());
    /// Set to make the plugin's next idle declare group 1's scope again
    /// while group 1 lives, as a plugin that reuses an id does.
    static REUSE: Cell<bool> = const { Cell::new(false) };
    /// The maker a plugin built with [`Setup::lend_maker`] lent.
    static MAKER: RefCell<Option<Maker>> = const { RefCell::new(None) };
    /// A block the next [`Racing`] declare loads, and the saved state it
    /// loads into.
    static LOAD_INSIDE: RefCell<Option<(Arc<SavedState>, Vec<u8>)>> =
        const { RefCell::new(None) };
}

/// The tree as the session saves it: its structure only, each group's id
/// and the next id. Seam saves each parameter's value, read live at the
/// save, and its learned mapping, under the group's scope.
#[derive(Debug)]
struct Tree {
    groups: List<Group>,
    maker: Maker,
    /// Only grows, and is saved with the tree, so no id is used twice while
    /// undo can bring a group back.
    next: Rc<Cell<u64>>,
    keep: bool,
}

impl Participant for Tree {
    fn capture(&self) -> Section {
        let ids: Vec<u64> = self
            .groups
            .with(|groups| groups.iter().map(|group| group.id).collect());
        json!({ "groups": ids, "next": self.next.get() })
    }

    fn apply(&mut self, section: Option<&Section>) -> Result<(), ApplyError> {
        let field = |name| section.and_then(|section: &Section| section.get(name));
        let ids: Vec<u64> = field("groups")
            .and_then(Json::as_array)
            .map(|ids| ids.iter().filter_map(Json::as_u64).collect())
            .unwrap_or_default();
        self.next
            .set(field("next").and_then(Json::as_u64).unwrap_or(1));
        let kept: Vec<(u64, GroupParams)> = if self.keep {
            self.groups.with(|groups| {
                groups
                    .iter()
                    .map(|group| (group.id, group.params.clone()))
                    .collect()
            })
        } else {
            Vec::new()
        };
        // A group declared again under its scope is born holding its saved
        // values, and learned again.
        let groups = ids
            .into_iter()
            .map(|id| {
                let params = kept
                    .iter()
                    .find(|(kept, _)| *kept == id)
                    .map(|(_, params)| params.clone())
                    .unwrap_or_else(|| self.maker.declare(&scope(id)));
                Group { id, params }
            })
            .collect();
        self.groups.reset(groups);
        Ok(())
    }
}

#[derive(Debug)]
struct Mixer {
    parts: Parts,
    params: MixParams,
    /// A number the DSP reads that no host is shown, made once.
    drive: Float,
    history: History,
    groups: List<Group>,
    tree: Rc<RefCell<Tree>>,
    registration: Registration,
    next: Rc<Cell<u64>>,
    /// The list's change count the tree was last saved at.
    saved_at: u64,
    /// A group declared under a scope a live group has, by [`REUSE`].
    stray: Option<GroupParams>,
    bank: Bank,
    audio: Option<BankAudio>,
}

impl Mixer {
    fn add_group(&mut self) -> u64 {
        let id = self.next.get();
        self.next.set(id + 1);
        // Nothing to handle, and no Result: the declarations were checked
        // while compiling.
        let params: GroupParams = self.parts.declare(&scope(id));
        self.groups.push(Group { id, params });
        id
    }

    fn group(&self, id: u64) -> Option<GroupParams> {
        self.groups.with(|groups| {
            groups
                .iter()
                .find(|group| group.id == id)
                .map(|group| group.params.clone())
        })
    }

    fn ids(&self) -> Vec<u64> {
        self.groups
            .with(|groups| groups.iter().map(|group| group.id).collect())
    }

    fn remove_group(&mut self, id: u64) -> bool {
        let at = self
            .groups
            .with(|groups| groups.iter().position(|group| group.id == id));
        at.is_some_and(|at| self.groups.remove(at))
    }

    /// The controller number a parameter is learned to, if any.
    fn learned(&self, param: &Float) -> Option<Control> {
        let learn = self.parts.learn();
        learn.mapping(param).map(|mapping| mapping.source.control)
    }
}

impl Plugin for Mixer {
    const INFO: PluginInfo = PluginInfo {
        name: "Run-time mixer",
        vendor: "Seam",
        url: "",
        email: "",
        description: "",
        version: "0.1.0",
        id: "dev.seam.test.run-time-mixer",
        vst3_class_id: [7; 16],
        au_codes: AuCodes {
            subtype: *b"RtMx",
            manufacturer: *b"Seam",
        },
        category: Category::Effect,
        features: &[],
        layouts: &[Layout::STEREO],
        midi_input: true,
        midi_output: false,
    };

    type Params = MixParams;

    type Processor = MixerProcessor;

    fn new(params: MixParams, mut parts: Parts) -> Self {
        let setup = SETUP.get();
        let history = parts.undo(Bound::default());
        let groups = List::new(Vec::new(), parts.set());
        history.enroll_list(&groups, "Groups").expect("a new list");
        let next = Rc::new(Cell::new(1));
        let tree = Rc::new(RefCell::new(Tree {
            groups: groups.clone(),
            maker: parts.maker(),
            next: Rc::clone(&next),
            keep: setup.keep,
        }));
        let mut config = Config::default();
        config.max_mappings = setup.max_mappings;
        let register = |parts: &Parts| parts.session().register("tree", &tree).expect("a free key");
        let registration = if setup.tree_first {
            let registration = register(&parts);
            parts.midi_learn(config);
            registration
        } else {
            parts.midi_learn(config);
            register(&parts)
        };
        let drive = parts.add("", DRIVE);
        if setup.lend_maker {
            MAKER.set(Some(parts.maker()));
        }
        let (bank, audio) = bank();
        let mut mixer = Mixer {
            parts,
            params,
            drive,
            history,
            groups,
            tree,
            registration,
            next,
            saved_at: 0,
            stray: None,
            bank,
            audio: Some(audio),
        };
        mixer.add_group();
        mixer.idle();
        // The first group is no step: the history starts with the plugin.
        mixer.history.clear().expect("not replaying");
        mixer
    }

    fn activate(&mut self, _setup: &ProcessSetup) -> MixerProcessor {
        MixerProcessor {
            params: self.params.clone(),
            drive: self.drive.clone(),
            changes: Changes::new(),
            gain: 1.0,
            bank: self.audio.take().expect("one activation at a time"),
        }
    }

    fn deactivate(&mut self, processor: MixerProcessor) {
        self.audio = Some(processor.bank);
    }

    fn idle(&mut self) {
        if REUSE.take()
            && let Some(first) = self.group(1)
        {
            self.parts.ui().write(&first.volume, -5.0);
            self.stray = Some(self.parts.declare(&scope(1)));
        }
        // Structure moves rarely, and the list's count says when: an add, a
        // remove, their undo, a load.
        let changes = self.groups.changes();
        if changes != self.saved_at {
            self.saved_at = changes;
            self.registration.changed();
        }
        self.bank.follow(&self.groups);
    }
}

#[derive(Debug)]
struct MixerProcessor {
    params: MixParams,
    drive: Float,
    changes: Changes,
    gain: f32,
    bank: BankAudio,
}

impl Processor for MixerProcessor {
    fn process(&mut self, block: Block<'_, '_>) -> Status {
        self.bank.sync();
        if self.changes.moved([&self.params.gain, &self.drive]) {
            self.gain = factor(self.params.gain.get()) * (1.0 + self.drive.get());
        }
        let level = self
            .bank
            .groups
            .iter_mut()
            .fold(self.gain, |level, group| level * group.retune());
        for channel in block.channels.iter_mut() {
            for sample in channel.iter_mut() {
                *sample *= level;
            }
        }
        Status::Continue
    }
}

/// The plugin as an exporter builds it, its driver, and its saved state.
fn built() -> (Mixer, Driver, Arc<SavedState>) {
    let (params, parts) = Parts::for_plugin::<Mixer>().expect("valid declarations");
    let driver = Driver::of(&parts);
    let saved = Arc::clone(parts.saved());
    (Mixer::new(params, parts), driver, saved)
}

/// [`built`], with `setup`.
fn built_with(setup: Setup) -> (Mixer, Driver, Arc<SavedState>) {
    SETUP.set(setup);
    let built = built();
    SETUP.set(Setup::default());
    built
}

/// One tick as the engine runs it: the parts, then the plugin's idle.
fn tick(plugin: &mut Mixer, driver: &Driver) {
    driver.tick();
    plugin.idle();
}

fn activated(plugin: &mut Mixer) -> MixerProcessor {
    plugin.activate(&ProcessSetup {
        sample_rate: 48_000.0,
        max_frames: 8,
        layout: Layout::STEREO,
        max_events: 4,
        max_out_events: 0,
    })
}

/// One block of 1.0s under the audio-thread mark and the allocation gate;
/// returns the level it came out at.
fn block(processor: &mut MixerProcessor, events: &EventList) -> f32 {
    let (mut left, mut right) = ([1.0_f32; 8], [1.0_f32; 8]);
    let mut out = EventList::with_capacity(0);
    let mut channels = [&mut left[..], &mut right[..]];
    let block = Block::new(&mut channels, 8, events, &mut out);
    seam_testkit::alloc::assert_no_allocations("a block", || {
        let _mark = audio_thread::enter();
        Processor::process(processor, block);
    });
    left[0]
}

fn cc(controller: u8, value: u8) -> EventList {
    let mut events = EventList::with_capacity(1);
    let voice = Voice::Midi1ControlChange { controller, value };
    events.push(
        ChannelVoice {
            group: 0,
            channel: 0,
            voice,
        }
        .event(0),
    );
    events
}

/// A mapping of controller `number` on any channel.
fn on(number: u8) -> Mapping {
    Mapping::new(Source::any(Control::Cc(number)))
}

fn close(a: f32, b: f32) -> bool {
    (a - b).abs() < 1e-5
}

/// The `runTime` object of a saved block.
fn run_time(bytes: &[u8]) -> Json {
    let block: Json = serde_json::from_slice(bytes).expect("a JSON block");
    block["runTime"].clone()
}

/// A mixer with groups `ids`, each group's volume at `-(id)` dB, the first
/// learned to controller 20 + id, saved.
fn saved_block(ids: &[u64]) -> Vec<u8> {
    let (mut plugin, driver, saved) = built();
    plugin.history.clear().expect("not replaying");
    let last = ids.iter().copied().max().unwrap_or(1);
    while plugin.next.get() <= last {
        plugin.add_group();
    }
    for id in plugin.ids() {
        if !ids.contains(&id) {
            assert!(plugin.remove_group(id));
        }
    }
    for &id in ids {
        let group = plugin.group(id).expect("added");
        assert!(plugin.parts.ui().write(&group.volume, -(id as f32)));
    }
    let first = plugin.group(ids[0]).expect("added");
    let number = 20 + ids[0] as u8;
    plugin
        .parts
        .learn()
        .map(&first.volume, on(number))
        .expect("legal");
    tick(&mut plugin, &driver);
    saved.save()
}

#[test]
fn never_a_host_parameter() {
    let vst3 = seam_host::vst3::TestHost::<Mixer>::new();
    assert_eq!(vst3.parameter_count(), 1);
    for key in ["volume", "muted", "drive"] {
        assert!(vst3.set_normalized(host_id(key), 0.5).is_err(), "{key}");
    }
    assert!(vst3.set_normalized(host_id("gain"), 0.5).is_ok());

    let cabi = seam_host::cabi::TestHost::<Mixer>::new();
    assert_eq!(cabi.parameter_count(), 1);
    assert!(cabi.parameter(1).is_none());

    let (plugin, _, _) = built();
    let set = plugin.parts.set();
    assert_eq!(set.len(), 1);
    assert_eq!(set.by_host_id(host_id("volume")), None);
    assert_eq!(plugin.groups.len(), 1, "the group exists, unlisted");
}

#[test]
fn instant_a_write_reaches_the_next_block_with_no_tick() {
    let (mut plugin, driver, _) = built();
    let mut processor = activated(&mut plugin);
    let none = EventList::with_capacity(0);
    assert!(close(block(&mut processor, &none), 1.0));

    let first = plugin.group(1).expect("the first group");
    assert!(plugin.parts.ui().write(&first.volume, -6.0));
    assert!(close(block(&mut processor, &none), factor(-6.0)));
    assert!(plugin.parts.ui().write(&first.muted, true));
    assert_eq!(block(&mut processor, &none), 0.0);
    assert!(plugin.parts.ui().write(&first.muted, false));
    assert!(plugin.parts.ui().write(&plugin.drive, 0.5));
    assert!(close(block(&mut processor, &none), factor(-6.0) * 1.5));

    // A new group reaches the audio side at the tick: structure may take
    // one, its knobs may not.
    let second = plugin.add_group();
    tick(&mut plugin, &driver);
    let second = plugin.group(second).expect("added");
    assert!(plugin.parts.ui().write(&second.volume, -6.0));
    assert!(close(block(&mut processor, &none), factor(-12.0) * 1.5));
    plugin.deactivate(processor);
}

#[test]
fn undoable_add_edit_remove_in_one_history() {
    let (mut plugin, driver, _) = built();
    let ui = plugin.parts.ui().clone();
    let history = plugin.history.clone();
    let gain = plugin.params.gain.clone();

    let id = plugin.add_group();
    tick(&mut plugin, &driver);
    let group = plugin.group(id).expect("added");
    assert!(ui.write(&group.volume, -9.0));
    assert!(ui.write(&gain, -3.0));
    {
        let drag = ui.begin(&group.volume);
        drag.write(-10.0);
        drag.write(-12.0);
    }
    tick(&mut plugin, &driver);
    assert!(plugin.remove_group(id));
    tick(&mut plugin, &driver);
    let names: Vec<String> = history.entries().into_iter().map(|e| e.name).collect();
    assert_eq!(names, ["Groups", "Volume", "Gain", "Volume", "Groups"]);

    // The removal undone: the same group, the same stores, the same value.
    assert_eq!(history.undo(), Ok(true));
    let back = plugin.group(id).expect("back");
    assert_eq!(back.volume, group.volume, "the same parameter, not a copy");
    assert_eq!(back.volume.get(), -12.0);
    assert_eq!(history.undo(), Ok(true));
    assert_eq!(group.volume.get(), -9.0, "the drag undone");
    assert_eq!(history.undo(), Ok(true));
    assert_eq!(gain.get(), 0.0);
    assert_eq!(history.undo(), Ok(true));
    assert_eq!(group.volume.get(), 0.0);
    assert_eq!(history.undo(), Ok(true));
    assert!(plugin.group(id).is_none(), "the add undone");
    assert_eq!(history.redo(), Ok(true));
    assert_eq!(history.redo(), Ok(true));
    assert_eq!(plugin.group(id).expect("redone").volume.get(), -9.0);
}

#[test]
fn a_step_never_keeps_a_parameter_alive() {
    let (mut plugin, driver, _) = built();
    let mut processor = activated(&mut plugin);
    let none = EventList::with_capacity(0);
    block(&mut processor, &none);
    let ui = plugin.parts.ui().clone();
    let first = plugin.group(1).expect("the first group");
    let store = first.volume.erased().downgrade();
    assert!(ui.write(&first.volume, -6.0));
    tick(&mut plugin, &driver);
    assert_eq!(plugin.history.len(), 1);

    // Gone with no step to bring it back: a reset is a restore's write. The
    // audio side gives its half back at the next block, and the main
    // thread drops it at the tick after.
    drop(first);
    plugin.groups.reset(Vec::new());
    tick(&mut plugin, &driver);
    assert!(store.upgrade().is_some(), "the audio side still holds it");
    block(&mut processor, &none);
    tick(&mut plugin, &driver);
    assert!(store.upgrade().is_none(), "freed once the plugin let it go");
    // The step is still listed, and replays as nothing.
    assert_eq!(plugin.history.len(), 1);
    assert_eq!(plugin.history.undo(), Ok(true));
    plugin.deactivate(processor);
}

#[test]
fn midi_learnable_in_the_block_the_controller_arrives_in() {
    let (mut plugin, driver, saved) = built();
    let mut processor = activated(&mut plugin);
    let mut learn = driver.learn_audio().expect("learn is on");
    let mut run = |processor: &mut MixerProcessor, events: &EventList| {
        seam_testkit::alloc::assert_no_allocations("learn's audio side", || {
            let _mark = audio_thread::enter();
            learn.process(events);
        });
        block(processor, events)
    };

    let first = plugin.group(1).expect("the first group");
    plugin
        .parts
        .learn()
        .arm(&first.volume, Takes::CONTROLLER)
        .expect("a parameter made at run time is learnable");
    run(&mut processor, &cc(7, 64));
    tick(&mut plugin, &driver);
    assert_eq!(plugin.learned(&first.volume), Some(Control::Cc(7)));

    // Controller 7 at 0: -60 dB, in its own block.
    let level = run(&mut processor, &cc(7, 0));
    assert!(close(level, factor(-60.0)), "{level}");
    assert_eq!(first.volume.get(), -60.0);
    tick(&mut plugin, &driver);
    assert_eq!(plugin.history.len(), 0, "a learned move is no step");

    // Saved under the group's scope, beside the value, and not in learn's
    // own section; learned again when a group of that scope is declared
    // after a load.
    let bytes = saved.save();
    let entry = &run_time(&bytes)["group/1/volume"];
    assert_eq!(entry["value"], -60.0);
    assert_eq!(entry["learned"]["number"], 7);
    let block_json: Json = serde_json::from_slice(&bytes).expect("JSON");
    assert_eq!(block_json["extra"]["midiLearn"]["mappings"], json!([]));
    let (mut reopened, driver, saved) = built();
    let mut processor_again = activated(&mut reopened);
    saved.load(&bytes).expect("a saved block");
    tick(&mut reopened, &driver);
    let again = reopened.group(1).expect("loaded");
    assert_eq!(again.volume.get(), -60.0);
    assert_eq!(reopened.learned(&again.volume), Some(Control::Cc(7)));
    let none = EventList::with_capacity(0);
    block(&mut processor_again, &none);

    // A group the plugin lets go is unmapped at the poll after its last
    // holder drops, and freed once learn's tables let go too.
    let store = again.volume.erased().downgrade();
    drop(again);
    reopened.groups.reset(Vec::new());
    tick(&mut reopened, &driver); // the retire order goes out
    block(&mut processor_again, &none); // the audio half comes back
    tick(&mut reopened, &driver); // and drops here
    assert!(
        !reopened.parts.learn().is_empty(),
        "learn polled before the drop"
    );
    tick(&mut reopened, &driver);
    assert!(reopened.parts.learn().is_empty(), "let go at the next poll");
    assert!(
        store.upgrade().is_some(),
        "a table buffer still routes to it"
    );
    reopened.deactivate(processor_again);
    plugin.deactivate(processor);
    drop(learn);
    drop(reopened);
    assert!(store.upgrade().is_none());
}

#[test]
fn saved_groups_values_and_mappings_load_into_a_second_instance() {
    let (mut plugin, driver, saved) = built();
    let ui = plugin.parts.ui().clone();
    let second = plugin.add_group();
    let third = plugin.add_group();
    tick(&mut plugin, &driver);
    let (second_params, third_params) = (
        plugin.group(second).expect("added"),
        plugin.group(third).expect("added"),
    );
    assert!(ui.write(&second_params.volume, -6.5));
    assert!(ui.write(&third_params.muted, true));
    assert!(ui.write(&plugin.drive, 0.3));
    let mut learn = plugin.parts.learn();
    learn.map(&third_params.muted, on(9)).expect("legal");
    learn.map(&plugin.drive, on(10)).expect("legal");
    drop(learn);
    assert!(plugin.remove_group(1));
    tick(&mut plugin, &driver);
    let bytes = saved.save();

    let (mut reopened, driver, saved) = built();
    let mut processor = activated(&mut reopened);
    saved.load(&bytes).expect("a saved block");
    tick(&mut reopened, &driver);
    assert_eq!(reopened.ids(), [second, third]);
    let loaded = reopened.group(second).expect("loaded");
    assert_eq!(loaded.volume.get(), -6.5);
    let muted = reopened.group(third).expect("loaded").muted;
    assert!(muted.get());
    let learn = reopened.parts.learn();
    assert_eq!(learn.mapping(&muted).map(|m| m.source), Some(on(9).source));
    assert_eq!(reopened.drive.get(), 0.3);
    assert_eq!(
        learn.mapping(&reopened.drive).map(|m| m.source),
        Some(on(10).source)
    );
    drop(learn);
    assert_eq!(
        reopened.history.undo(),
        Ok(false),
        "a load leaves nothing to undo"
    );
    assert_eq!(
        block(&mut processor, &EventList::with_capacity(0)),
        0.0,
        "muted"
    );
    reopened.deactivate(processor);
}

#[test]
fn told_to_no_host() {
    let (plugin, _, _) = built();
    let set = Arc::clone(plugin.parts.set());
    let mut drain = set.take_host_event_drain().expect("the first drain");
    let first = plugin.group(1).expect("the first group");

    assert!(plugin.parts.ui().write(&first.volume, -3.0));
    assert!(plugin.parts.ui().write(&plugin.drive, 0.5));
    let mut heard = Vec::new();
    drain.drain(|event| heard.push(event));
    assert!(heard.is_empty(), "{heard:?}");

    assert!(plugin.parts.ui().write(&plugin.params.gain, -3.0));
    drain.drain(|event| heard.push(event));
    assert_eq!(heard.len(), 3, "a listed edit is heard: {heard:?}");
}

#[test]
fn nothing_to_handle_and_a_source_mistake_panics_with_the_fix() {
    let (plugin, _, _) = built();
    let one: Float = plugin.parts.add("lone", VOLUME);
    assert_eq!(one.get(), 0.0);

    #[derive(Debug)]
    struct Twice(
        #[expect(dead_code, reason = "declared for the panic")] Float,
        #[expect(dead_code, reason = "declared for the panic")] Float,
    );
    impl Params for Twice {
        fn declare(p: &mut Declarer) -> Self {
            Twice(p.add(VOLUME), p.add(VOLUME))
        }
    }
    let message = seam_testkit::contract::expect_contract_panic("declared twice", || {
        let _: Twice = plugin.parts.declare("twice");
    });
    assert!(message.contains("\"volume\""), "{message}");

    #[derive(Debug)]
    struct Bypass(#[expect(dead_code, reason = "declared for the panic")] Bool);
    impl Params for Bypass {
        fn declare(p: &mut Declarer) -> Self {
            Bypass(p.add(BoolDecl::new("bypass", "Bypass", false).as_bypass()))
        }
    }
    seam_testkit::contract::expect_contract_panic(
        "declare the bypass in the plugin's Params",
        || {
            let _: Bypass = plugin.parts.declare("bypass");
        },
    );

    #[derive(Debug)]
    struct Table;
    impl Params for Table {
        fn declare(p: &mut Declarer) -> Self {
            p.extend([seam_params::ParamDecl::new(
                "on",
                "On",
                seam_params::Kind::Bool,
                0.0,
            )]);
            Table
        }
    }
    seam_testkit::contract::expect_contract_panic("declare each with Declarer::add", || {
        let _: Table = plugin.parts.maker().declare("table");
    });
}

#[test]
fn a_declare_after_a_load_is_born_holding_the_saved_value() {
    let bytes = saved_block(&[1, 3]);
    let (mut plugin, driver, saved) = built();
    saved.load(&bytes).expect("a saved block");
    tick(&mut plugin, &driver);
    let third = plugin.group(3).expect("loaded");
    let reading = third.volume.erased().read();
    assert_eq!(
        (reading.value, reading.origin, reading.changes),
        (-3.0, Origin::Restore, 0),
        "its first reading is the saved value, unwritten"
    );
}

#[test]
fn a_private_parameter_made_once_is_restored_at_the_tick_that_applies_a_load() {
    let (mut source, driver, saved) = built();
    assert!(source.parts.ui().write(&source.drive, 0.7));
    source
        .parts
        .learn()
        .map(&source.drive, on(9))
        .expect("legal");
    tick(&mut source, &driver);
    let bytes = saved.save();

    let (mut plugin, driver, saved) = built();
    assert!(plugin.parts.ui().write(&plugin.drive, 0.2));
    saved.load(&bytes).expect("a saved block");
    assert_eq!(
        plugin.drive.get(),
        0.2,
        "until the tick, it reads as it did"
    );
    tick(&mut plugin, &driver);
    assert_eq!(plugin.drive.get(), 0.7);
    assert_eq!(plugin.drive.erased().read().origin, Origin::Restore);
    assert_eq!(plugin.learned(&plugin.drive), Some(Control::Cc(9)));

    // A block without it sets it to its default, unlearned.
    let mut lacking: Json = serde_json::from_slice(&bytes).expect("JSON");
    lacking["runTime"]
        .as_object_mut()
        .expect("an object")
        .remove("drive");
    let lacking = serde_json::to_vec(&lacking).expect("JSON");
    saved.load(&lacking).expect("a saved block");
    assert_eq!(plugin.drive.get(), 0.7);
    tick(&mut plugin, &driver);
    assert_eq!(plugin.drive.get(), 0.0);
    assert_eq!(plugin.learned(&plugin.drive), None);
}

#[test]
fn a_load_into_a_running_instance_writes_none_of_the_old_tree() {
    let bytes = saved_block(&[1, 3]);
    let (mut plugin, driver, saved) = built();
    let mut processor = activated(&mut plugin);
    plugin.add_group();
    tick(&mut plugin, &driver);
    let old = [
        plugin.group(1).expect("first"),
        plugin.group(2).expect("second"),
    ];
    assert!(plugin.parts.ui().write(&old[0].volume, -20.0));
    assert!(plugin.parts.ui().write(&old[1].muted, true));
    let readings = |groups: &[GroupParams; 2]| -> Vec<(u64, f32)> {
        groups
            .iter()
            .flat_map(|group| [group.volume.erased(), group.muted.erased()])
            .map(|param| param.read())
            .map(|reading| (reading.changes, reading.value))
            .collect()
    };
    let before = readings(&old);

    // Group 1 is in the block and group 2 is not: until the tick, both keep
    // what they hold, and the old tree plays on.
    saved.load(&bytes).expect("a saved block");
    assert_eq!(readings(&old), before);
    assert_eq!(
        block(&mut processor, &EventList::with_capacity(0)),
        0.0,
        "still muted"
    );

    tick(&mut plugin, &driver);
    assert_eq!(plugin.ids(), [1, 3]);
    let first = plugin.group(1).expect("loaded");
    assert_ne!(first.volume, old[0].volume, "declared again, a new store");
    assert_eq!((first.volume.get(), old[0].volume.get()), (-1.0, -20.0));
    plugin.deactivate(processor);
}

#[test]
fn an_owner_the_apply_kept_takes_the_blocks_values_and_mappings() {
    let bytes = saved_block(&[1]);
    let (mut plugin, driver, saved) = built_with(Setup {
        keep: true,
        ..Setup::default()
    });
    let kept = plugin.group(1).expect("the first group");
    assert!(plugin.parts.ui().write(&kept.volume, -30.0));
    plugin.parts.learn().map(&kept.muted, on(3)).expect("legal");
    tick(&mut plugin, &driver);

    saved.load(&bytes).expect("a saved block");
    tick(&mut plugin, &driver);
    let group = plugin.group(1).expect("kept");
    assert_eq!(group.volume, kept.volume, "the same store");
    assert_eq!(kept.volume.get(), -1.0);
    assert_eq!(plugin.learned(&kept.volume), Some(Control::Cc(21)));
    let learn = plugin.parts.learn();
    assert_eq!(
        learn.mapping(&kept.muted),
        None,
        "the block maps it to nothing"
    );
}

#[test]
fn a_held_up_load_still_reaches_its_owners_and_then_lets_its_entries_go() {
    let bytes = {
        let (mut source, driver, saved) = built();
        let lone: Float = source.parts.add("lone", VOLUME);
        assert!(source.parts.ui().write(&lone, -20.0));
        source.add_group();
        let second = source.group(2).expect("added");
        assert!(source.parts.ui().write(&second.volume, -2.0));
        tick(&mut source, &driver);
        saved.save()
    };
    let (mut plugin, driver, saved) = built();
    saved.load(&bytes).expect("a saved block");

    // The tree is busy through the load's first tick.
    let busy = plugin.tree.borrow();
    driver.tick();
    drop(busy);
    assert_eq!(plugin.ids(), [1], "not applied yet");
    let lone: Float = plugin.parts.add("lone", VOLUME);
    assert_eq!(lone.get(), -20.0, "the entries wait for the pass");
    drop(lone);

    tick(&mut plugin, &driver);
    assert_eq!(plugin.ids(), [1, 2]);
    assert_eq!(plugin.group(2).expect("loaded").volume.get(), -2.0);
    let late: Float = plugin.parts.add("lone", VOLUME);
    assert_eq!(
        late.get(),
        0.0,
        "the pass finished: a later declare gets defaults"
    );
}

/// A participant that loads `block` into `saved` once: a load the host
/// stores while the main thread's tick runs, pinned at one point. From its
/// apply, inside the session's pass; or, `in_capture`, from the capture the
/// session's tick makes of a held-up participant before it takes the pass.
struct Racer {
    saved: Arc<SavedState>,
    block: RefCell<Option<Vec<u8>>>,
    in_capture: bool,
}

impl Racer {
    fn new(
        saved: &Arc<SavedState>,
        block: Option<Vec<u8>>,
        in_capture: bool,
    ) -> Rc<RefCell<Racer>> {
        Rc::new(RefCell::new(Racer {
            saved: Arc::clone(saved),
            block: RefCell::new(block),
            in_capture,
        }))
    }

    fn load(&self) {
        if let Some(block) = self.block.take() {
            self.saved.load(&block).expect("a saved block");
        }
    }
}

impl Participant for Racer {
    fn capture(&self) -> Section {
        if self.in_capture {
            self.load();
        }
        Section::Null
    }

    fn apply(&mut self, _section: Option<&Section>) -> Result<(), ApplyError> {
        if !self.in_capture {
            self.load();
        }
        Ok(())
    }
}

#[test]
fn a_load_stored_while_a_pass_runs_waits_for_a_pass_that_began_after_it() {
    let first = saved_block(&[1]);
    let second = saved_block(&[1, 4]);
    let (mut plugin, driver, saved) = built();
    let racer = Racer::new(&saved, Some(second), false);
    let _racer = plugin
        .parts
        .session()
        .register("racer", &racer)
        .expect("a free key");

    // The tree applies the first block, then the racer stores the second.
    saved.load(&first).expect("a saved block");
    tick(&mut plugin, &driver);
    assert!(racer.borrow().block.borrow().is_none(), "the racer loaded");
    assert_eq!(plugin.ids(), [1]);
    assert_eq!(plugin.group(1).expect("loaded").volume.get(), -1.0);

    // The pass that began before the second load did not let its entries
    // go: the next pass's declares are born holding them.
    tick(&mut plugin, &driver);
    assert_eq!(plugin.ids(), [1, 4]);
    assert_eq!(plugin.group(4).expect("loaded").volume.get(), -4.0);
    assert_eq!(
        plugin.learned(&plugin.group(1).expect("loaded").volume),
        Some(Control::Cc(21))
    );
}

#[test]
fn a_save_between_a_load_and_its_tick_writes_the_loaded_section() {
    let bytes = saved_block(&[1, 3]);
    let loaded = run_time(&bytes);

    // From a fresh instance, and from a running one whose old tree shares a
    // path with the block.
    let (_fresh, _, fresh) = built();
    let (mut running, driver, saved) = built();
    running.add_group();
    tick(&mut running, &driver);
    for group in [1, 2] {
        let group = running.group(group).expect("added");
        assert!(running.parts.ui().write(&group.volume, -40.0));
    }
    for saved in [&fresh, &saved] {
        saved.load(&bytes).expect("a saved block");
        let again = saved.save();
        assert_eq!(run_time(&again), loaded, "none of the old tree's values");

        // Loaded into a fresh instance and ticked: the first block's state.
        let (mut reopened, driver, other) = built();
        other.load(&again).expect("a saved block");
        tick(&mut reopened, &driver);
        assert_eq!(reopened.ids(), [1, 3]);
        let first = reopened.group(1).expect("loaded");
        assert_eq!(first.volume.get(), -1.0);
        assert_eq!(reopened.group(3).expect("loaded").volume.get(), -3.0);
        assert_eq!(reopened.learned(&first.volume), Some(Control::Cc(21)));
    }
}

#[test]
fn a_load_into_a_learned_instance_at_its_ceiling_brings_back_every_mapping() {
    let setup = Setup {
        max_mappings: 2,
        ..Setup::default()
    };
    // The block's ids the old tree's, and others.
    for ids in [[1, 2], [3, 4]] {
        let bytes = {
            let (mut source, driver, saved) = built_with(setup);
            while source.next.get() <= ids[1] {
                source.add_group();
            }
            for id in source.ids() {
                if !ids.contains(&id) {
                    assert!(source.remove_group(id));
                }
            }
            for (number, id) in [11, 12].into_iter().zip(ids) {
                let group = source.group(id).expect("added");
                source
                    .parts
                    .learn()
                    .map(&group.volume, on(number))
                    .expect("legal");
            }
            tick(&mut source, &driver);
            saved.save()
        };
        let (mut plugin, driver, saved) = built_with(setup);
        plugin.add_group();
        for (number, id) in [1, 2].into_iter().zip([1, 2]) {
            let group = plugin.group(id).expect("added");
            plugin
                .parts
                .learn()
                .map(&group.volume, on(number))
                .expect("legal");
        }
        tick(&mut plugin, &driver);

        saved.load(&bytes).expect("a saved block");
        tick(&mut plugin, &driver);
        assert_eq!(plugin.ids(), ids);
        let learned: Vec<_> = ids
            .iter()
            .map(|&id| plugin.learned(&plugin.group(id).expect("loaded").volume))
            .collect();
        assert_eq!(
            learned,
            [Some(Control::Cc(11)), Some(Control::Cc(12))],
            "{ids:?}"
        );
        assert_eq!(
            plugin.parts.learn().len(),
            2,
            "none of the old tree's stays"
        );
    }
}

#[test]
fn the_tree_registered_before_or_after_learn_gives_the_same_load_at_the_ceiling() {
    // The block maps one group and no listed parameter. The instance it
    // replaces has its listed gain mapped, at a ceiling of one mapping.
    let bytes = saved_block(&[2, 3]);
    let loaded = [false, true].map(|tree_first| {
        let (mut plugin, driver, saved) = built_with(Setup {
            tree_first,
            max_mappings: 1,
            ..Setup::default()
        });
        plugin
            .parts
            .learn()
            .map(&plugin.params.gain, on(7))
            .expect("legal");
        tick(&mut plugin, &driver);
        saved.load(&bytes).expect("a saved block");
        tick(&mut plugin, &driver);
        let groups: Vec<_> = plugin
            .ids()
            .into_iter()
            .map(|id| {
                let group = plugin.group(id).expect("loaded");
                (id, group.volume.get(), plugin.learned(&group.volume))
            })
            .collect();
        (groups, plugin.learned(&plugin.params.gain))
    });
    assert_eq!(loaded[0], loaded[1]);
    let (groups, gain) = &loaded[0];
    assert_eq!(groups[0], (2, -2.0, Some(Control::Cc(22))));
    assert_eq!(*gain, None, "the block maps no listed parameter");
}

/// A struct whose declare a load lands inside, after its parameter is made
/// and before it is named: a host's load on another thread, pinned there.
struct Racing {
    volume: Float,
}

impl Params for Racing {
    fn declare(p: &mut Declarer) -> Self {
        let volume = p.add(VOLUME);
        if let Some((saved, block)) = LOAD_INSIDE.take() {
            saved.load(&block).expect("a saved block");
        }
        Racing { volume }
    }
}

#[test]
fn a_declare_a_load_lands_inside_counts_as_declared_before_the_load() {
    let bytes = {
        let (source, _driver, saved) = built();
        let lone: Float = source.parts.add("lone", VOLUME);
        assert!(source.parts.ui().write(&lone, -5.0));
        saved.save()
    };
    let (mut plugin, driver, saved) = built();
    LOAD_INSIDE.set(Some((Arc::clone(&saved), bytes.clone())));
    let racing: Racing = plugin.parts.declare("lone");
    assert_eq!(racing.volume.get(), 0.0, "born before the load");

    // The load's entry is what a save writes until its pass, and then what
    // the parameter takes.
    assert_eq!(run_time(&saved.save()), run_time(&bytes));
    tick(&mut plugin, &driver);
    let reading = racing.volume.erased().read();
    assert_eq!((reading.value, reading.origin), (-5.0, Origin::Restore));
}

#[test]
fn a_load_stored_in_the_tick_before_its_pass_brings_back_every_mapping() {
    let setup = Setup {
        max_mappings: 2,
        ..Setup::default()
    };
    // The block maps the listed gain and group 2.
    let bytes = {
        let (mut source, driver, saved) = built_with(setup);
        source.add_group();
        assert!(source.remove_group(1));
        let two = source.group(2).expect("added");
        let mut learn = source.parts.learn();
        learn.map(&source.params.gain, on(7)).expect("legal");
        learn.map(&two.volume, on(22)).expect("legal");
        drop(learn);
        tick(&mut source, &driver);
        saved.save()
    };
    let loaded = [false, true].map(|race| {
        // The instance has groups 1 and 3 mapped, at the ceiling.
        let (mut plugin, driver, saved) = built_with(setup);
        plugin.add_group();
        plugin.add_group();
        assert!(plugin.remove_group(2));
        for id in [1, 3] {
            let group = plugin.group(id).expect("added");
            let number = 20 + id as u8;
            plugin
                .parts
                .learn()
                .map(&group.volume, on(number))
                .expect("legal");
        }
        tick(&mut plugin, &driver);
        let racer = Racer::new(&saved, None, true);
        let registration = plugin
            .parts
            .session()
            .register("racer", &racer)
            .expect("a free key");
        if race {
            // Held up, so the next tick's session captures it, and it
            // loads, before the session takes its pass.
            let busy = racer.borrow_mut();
            registration.changed();
            drop(busy);
            racer.borrow().block.replace(Some(bytes.clone()));
        } else {
            saved.load(&bytes).expect("a saved block");
        }
        tick(&mut plugin, &driver);
        let two = plugin.group(2).expect("loaded");
        (
            plugin.ids(),
            plugin.learned(&plugin.params.gain),
            plugin.learned(&two.volume),
            plugin.parts.learn().len(),
        )
    });
    assert_eq!(
        loaded[0],
        (vec![2], Some(Control::Cc(7)), Some(Control::Cc(22)), 2)
    );
    assert_eq!(loaded[1], loaded[0], "a load stored in the tick");
}

/// A participant that saves the block from its apply: registered after the
/// tree, so a pass applies it after the tree.
struct Saver {
    saved: Arc<SavedState>,
    block: Option<Vec<u8>>,
}

impl Participant for Saver {
    fn capture(&self) -> Section {
        Section::Null
    }

    fn apply(&mut self, _section: Option<&Section>) -> Result<(), ApplyError> {
        self.block = Some(self.saved.save());
        Ok(())
    }
}

#[test]
fn a_save_inside_the_pass_holds_the_mapping_of_a_group_the_pass_declared() {
    let bytes = saved_block(&[2]);
    let (mut plugin, driver, saved) = built();
    let saver = Rc::new(RefCell::new(Saver {
        saved: Arc::clone(&saved),
        block: None,
    }));
    let _saver = plugin
        .parts
        .session()
        .register("saver", &saver)
        .expect("a free key");
    saved.load(&bytes).expect("a saved block");
    tick(&mut plugin, &driver);
    let inside = saver.borrow_mut().block.take().expect("the saver applied");
    let loaded = &run_time(&bytes)["group/2/volume"];
    assert!(loaded["learned"].is_object(), "the block maps group 2");
    assert_eq!(&run_time(&inside)["group/2/volume"], loaded);
    assert_eq!(&run_time(&saved.save())["group/2/volume"], loaded);
}

#[test]
fn a_shared_path_is_one_fault_and_saves_the_later_and_a_load_declaring_again_is_none() {
    let host = seam_host::vst3::TestHost::<Mixer>::new();
    REUSE.set(true);
    host.expect_faults(1);
    host.tick();
    host.tick(); // reports it
    let bytes = host.save().expect("saved");
    assert_eq!(
        run_time(&bytes)["group/1/volume"]["value"],
        0.0,
        "the later declare, not the first group's -5 dB"
    );

    // The load's apply declares group 1 again over the paths the plugin
    // holds: that is no fault, and the drop checks the count.
    host.load(&bytes).expect("loaded");
    host.tick();
    host.tick();
}

#[test]
fn a_declare_between_a_load_and_its_tick_is_no_shared_path_fault() {
    let bytes = saved_block(&[1, 5]);
    SETUP.set(Setup {
        lend_maker: true,
        ..Setup::default()
    });
    let host = seam_host::vst3::TestHost::<Mixer>::new();
    SETUP.set(Setup::default());
    let maker = MAKER.take().expect("the plugin lent its maker");
    host.load(&bytes).expect("loaded");

    // Declared outside the plugin's apply, under a path its pass declares
    // again, and kept: the pass replaces it as it replaces the old tree.
    let stray: GroupParams = maker.declare(&scope(5));
    assert_eq!(stray.volume.get(), -5.0, "born holding the load's value");
    host.tick();
    host.tick(); // a fault would be reported by now
    assert_eq!(host.faults(), 0);
    drop((stray, maker));
}

#[test]
fn a_block_that_repeats_an_owner_is_one_shared_path_fault() {
    let mut block: Json = serde_json::from_slice(&saved_block(&[2])).expect("JSON");
    block["extra"]["tree"]["groups"] = json!([2, 2]);
    let bytes = serde_json::to_vec(&block).expect("JSON");
    let host = seam_host::vst3::TestHost::<Mixer>::new();
    host.expect_faults(1);
    host.load(&bytes).expect("loaded");
    host.tick();
    host.tick(); // reports it
    let saved = host.save().expect("saved");
    assert_eq!(run_time(&saved)["group/2/volume"]["value"], -2.0);
}

#[test]
fn another_instances_parameter_is_refused_through_this_ones_ui() {
    let (ours, _, _) = built();
    let (theirs, _, _) = built();
    let group = theirs.group(1).expect("the first group");
    #[cfg(debug_assertions)]
    seam_testkit::contract::expect_contract_panic("a parameter of another set", || {
        ours.parts.ui().write(&group.volume, -6.0);
    });
    #[cfg(not(debug_assertions))]
    assert!(!ours.parts.ui().write(&group.volume, -6.0));
    assert_eq!(group.volume.get(), 0.0);
}
