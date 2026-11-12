//! The engine's rules, each tested through every shell with one body.

use std::cell::RefCell;
use std::error::Error;
use std::rc::Rc;

use seam_learn::{Config, LearnAudio, SESSION_KEY, Takes};
#[cfg(feature = "vst3")]
use seam_params::host_id;
use seam_params::{Declarer, Float, FloatDecl, Params, Value, audio_thread};
use seam_session::{ApplyError, Participant, Registration, Section};
use seam_ump::{ChannelVoice, EventList, Voice};
use seam_undo::{Bound, History};
use serde_json::{Map, Value as Json};

use crate::faults::written;
use crate::{
    AuCodes, Block, Category, Driver, Layout, Parts, Plugin, PluginInfo, ProcessSetup, Processor,
    Status, Tail, boundary_faults,
};

/// The parameters every plugin here declares: one gain, which hosts list
/// first.
#[derive(Clone)]
struct Gain {
    gain: Float,
}

impl Params for Gain {
    fn declare(p: &mut Declarer) -> Self {
        Gain {
            gain: p.add(FloatDecl::new("gain", "Gain", 0.0..=10.0, 0.0)),
        }
    }
}

/// What the probe saw, shared by the plugin and its processor, which the
/// tests run on the test's own thread.
#[derive(Default)]
struct Seen {
    activations: u32,
    deactivations: u32,
    resets: u32,
    panic_in_process: bool,
    /// Whether every block ran with the audio-thread mark.
    marked_blocks: Option<bool>,
    /// Processors dropped, and how many of them on the audio thread.
    drops: u32,
    drops_marked: u32,
    tail: Option<Tail>,
}

thread_local! {
    static SEEN: RefCell<Seen> = RefCell::default();
}

fn seen<R>(f: impl FnOnce(&mut Seen) -> R) -> R {
    SEEN.with(|seen| f(&mut seen.borrow_mut()))
}

/// A plugin whose processor adds its gain parameter to every sample. It
/// keeps the parameters it is handed and drops its parts.
struct Probe(Gain);

/// Reads the gain through the typed parameter `new` was handed.
struct Adder(Gain);

impl Processor for Adder {
    fn process(&mut self, block: Block<'_, '_>) -> Status {
        let marked = audio_thread::is_here();
        seen(|seen| seen.marked_blocks = Some(seen.marked_blocks.unwrap_or(true) && marked));
        assert!(!seen(|seen| seen.panic_in_process), "a processor's bug");
        let add = self.0.gain.get();
        for channel in block.channels.iter_mut() {
            for sample in &mut channel[..block.frames] {
                *sample += add;
            }
        }
        Status::Continue
    }

    fn reset(&mut self) {
        seen(|seen| seen.resets += 1);
    }
}

impl Drop for Adder {
    fn drop(&mut self) {
        let marked = audio_thread::is_here();
        seen(|seen| {
            seen.drops += 1;
            seen.drops_marked += u32::from(marked);
        });
    }
}

impl Plugin for Probe {
    const INFO: PluginInfo = PluginInfo {
        name: "Engine probe",
        vendor: "Seam",
        url: "",
        email: "",
        description: "",
        version: "1.0.0",
        id: "dev.seam.engine-probe",
        vst3_class_id: [7; 16],
        au_codes: AuCodes {
            subtype: *b"engp",
            manufacturer: *b"Seam",
        },
        category: Category::Effect,
        features: &[],
        layouts: &[Layout::STEREO],
        midi_input: false,
        midi_output: false,
    };
    type Params = Gain;
    type Processor = Adder;

    fn new(params: Gain, _: Parts) -> Self {
        seen(|seen| *seen = Seen::default());
        Probe(params)
    }

    fn activate(&mut self, _: &ProcessSetup) -> Adder {
        seen(|seen| seen.activations += 1);
        Adder(self.0.clone())
    }

    fn deactivate(&mut self, processor: Adder) {
        seen(|seen| seen.deactivations += 1);
        drop(processor);
    }

    fn tail(&self) -> Tail {
        seen(|seen| seen.tail).unwrap_or(Tail::None)
    }
}

/// A shell's test host, as the shared bodies drive it.
trait Shell {
    /// What the shell's host was answered when the plugin refused a call.
    type Refused: std::fmt::Debug + PartialEq;
    fn create() -> Self;
    /// Activates in stereo at 48 kHz for blocks of up to 64 frames.
    fn activate(&mut self) -> Result<(), Self::Refused>;
    fn deactivate(&mut self);
    /// Whatever the shell's host says as it starts processing.
    fn start(&mut self);
    /// Whatever the shell's host does to ask for a reset.
    fn ask_reset(&mut self);
    /// Whether the shell has a processing state that holds controller
    /// writes off.
    const HOLDS_WRITES_WHILE_PROCESSING: bool;
    /// A block of 8 frames of `input` in place; the left channel after it.
    fn play(&mut self, input: f32) -> [f32; 8];
    /// The same block on a thread of its own, as a host's audio thread,
    /// with its processor told to panic or not; the left channel after it,
    /// and what was written on that thread.
    fn play_elsewhere(&mut self, input: f32, panic: bool) -> ([f32; 8], Vec<String>);
    /// A write of the gain through the host's controller.
    fn write(&mut self, normalized: f64);
    fn gain(&self) -> f64;
    /// One main-thread tick.
    fn tick(&mut self);
    /// The tail the host reads.
    fn tail(&self) -> u32;
    /// The saved block, through the host's save.
    fn save(&self) -> Vec<u8>;
    /// The host's load of a block, and what it was answered.
    fn load(&mut self, block: &[u8]) -> Result<(), Self::Refused>;
    fn expect_faults(&self, count: u64);
    /// Whether the running processor came with learn's audio side; `None`
    /// with no processor.
    fn holds_learn_audio(&self) -> Option<bool>;
}

/// A shell's test host that can create an instance its plugin runs on
/// another thread.
trait Elsewhere: Shell {
    /// Creates the instance on another thread; this one calls it first.
    fn create_elsewhere() -> Self;
}

#[cfg(feature = "vst3")]
impl<P: Plugin> Elsewhere for crate::vst3::TestHost<P> {
    fn create_elsewhere() -> Self {
        Self::created_elsewhere()
    }
}

#[cfg(feature = "vst3")]
impl<P: Plugin> Shell for crate::vst3::TestHost<P> {
    type Refused = crate::vst3::Refused;
    const HOLDS_WRITES_WHILE_PROCESSING: bool = true;

    fn create() -> Self {
        Self::new()
    }

    fn activate(&mut self) -> Result<(), Self::Refused> {
        // Refused while active, as the SDK has it; the setup stays.
        let _ = self.setup(48_000.0, 64);
        crate::vst3::TestHost::activate(self)
    }

    fn deactivate(&mut self) {
        assert_eq!(crate::vst3::TestHost::deactivate(self), Ok(()));
    }

    fn start(&mut self) {
        assert_eq!(crate::vst3::TestHost::start(self), Ok(()));
    }

    fn ask_reset(&mut self) {
        assert_eq!(self.stop(), Ok(()));
        assert_eq!(crate::vst3::TestHost::start(self), Ok(()));
    }

    fn play(&mut self, input: f32) -> [f32; 8] {
        let (mut left, mut right) = ([input; 8], [input; 8]);
        let block = &mut [&mut left[..], &mut right[..]];
        assert_eq!(self.process_in_place(block, 8, &[]), Ok(()));
        left
    }

    fn play_elsewhere(&mut self, input: f32, panic: bool) -> ([f32; 8], Vec<String>) {
        let mut side = self.audio_side();
        on_another_thread(
            move || {
                let (mut left, mut right) = ([input; 8], [input; 8]);
                let block = &mut [&mut left[..], &mut right[..]];
                assert_eq!(side.process_in_place(block, 8, &[]), Ok(()));
                left
            },
            panic,
        )
    }

    fn write(&mut self, normalized: f64) {
        assert_eq!(self.set_normalized(host_id("gain"), normalized), Ok(()));
    }

    fn gain(&self) -> f64 {
        self.normalized(host_id("gain"))
    }

    fn tick(&mut self) {
        crate::vst3::TestHost::tick(self);
    }

    fn tail(&self) -> u32 {
        crate::vst3::TestHost::tail(self)
    }

    fn save(&self) -> Vec<u8> {
        crate::vst3::TestHost::save(self).expect("saved")
    }

    fn load(&mut self, block: &[u8]) -> Result<(), Self::Refused> {
        crate::vst3::TestHost::load(self, block)
    }

    fn expect_faults(&self, count: u64) {
        crate::vst3::TestHost::expect_faults(self, count);
    }

    fn holds_learn_audio(&self) -> Option<bool> {
        self.component().holds_learn_audio()
    }
}

#[cfg(feature = "cabi")]
impl<P: Plugin + Send> Elsewhere for crate::cabi::TestHost<P> {
    fn create_elsewhere() -> Self {
        Self::created_elsewhere()
    }
}

#[cfg(feature = "cabi")]
impl<P: Plugin> Shell for crate::cabi::TestHost<P> {
    type Refused = crate::cabi::Refused;
    const HOLDS_WRITES_WHILE_PROCESSING: bool = false;

    fn create() -> Self {
        Self::new()
    }

    fn activate(&mut self) -> Result<(), Self::Refused> {
        crate::cabi::TestHost::activate(self, 48_000.0, 64, Layout::STEREO)
    }

    fn deactivate(&mut self) {
        crate::cabi::TestHost::deactivate(self);
    }

    fn start(&mut self) {}

    fn ask_reset(&mut self) {
        self.reset();
    }

    fn play(&mut self, input: f32) -> [f32; 8] {
        let (mut left, mut right) = ([input; 8], [input; 8]);
        self.process_in_place(&mut [&mut left, &mut right], 8);
        left
    }

    fn play_elsewhere(&mut self, input: f32, panic: bool) -> ([f32; 8], Vec<String>) {
        let side = self.audio_side();
        on_another_thread(
            move || {
                let (mut left, mut right) = ([input; 8], [input; 8]);
                side.process_in_place(&mut [&mut left, &mut right], 8);
                left
            },
            panic,
        )
    }

    fn write(&mut self, normalized: f64) {
        self.set(0, normalized);
    }

    fn gain(&self) -> f64 {
        self.get(0)
    }

    fn tick(&mut self) {
        self.idle();
    }

    fn tail(&self) -> u32 {
        crate::cabi::TestHost::tail(self)
    }

    fn save(&self) -> Vec<u8> {
        crate::cabi::TestHost::save(self).expect("saved")
    }

    fn load(&mut self, block: &[u8]) -> Result<(), Self::Refused> {
        crate::cabi::TestHost::load(self, block)
    }

    fn expect_faults(&self, count: u64) {
        crate::cabi::TestHost::expect_faults(self, count);
    }

    fn holds_learn_audio(&self) -> Option<bool> {
        crate::cabi::TestHost::holds_learn_audio(self)
    }
}

/// Runs `block` on a thread of its own with its processor told to panic or
/// not, and answers its result and what was written on that thread.
fn on_another_thread(
    block: impl FnOnce() -> [f32; 8] + Send,
    panic: bool,
) -> ([f32; 8], Vec<String>) {
    std::thread::scope(|scope| {
        scope
            .spawn(move || {
                seen(|seen| seen.panic_in_process = panic);
                written();
                let left = block();
                (left, written())
            })
            .join()
            .expect("the audio thread")
    })
}

/// Runs each shared body through every shell built, with `$plugin`.
macro_rules! through_every_shell {
    ($plugin:ident: $($body:ident),* $(,)?) => {
        $(
            mod $body {
                #[cfg(feature = "vst3")]
                #[test]
                fn vst3() {
                    super::$body::<crate::vst3::TestHost<super::$plugin>>();
                }

                #[cfg(feature = "cabi")]
                #[test]
                fn cabi() {
                    super::$body::<crate::cabi::TestHost<super::$plugin>>();
                }
            }
        )*
    };
}

through_every_shell!(
    Probe:
    the_parameters_new_is_handed_are_the_ones_the_host_writes,
    a_plugin_created_on_one_thread_runs_on_the_thread_that_calls_it_first,
    an_activation_while_active_rebuilds_and_recovers_a_panicked_processor,
    a_block_with_no_processor_is_silent,
    a_reset_runs_before_the_next_block_unless_an_activation_replaced_the_processor,
    a_controller_write_while_processing_is_held_off_only_where_the_host_sends_it_again,
    a_fault_on_the_audio_path_is_reported_at_the_next_tick_once_per_instance_and_kind,
    the_block_runs_marked_as_the_audio_thread_and_no_processor_drops_there,
    created_on_one_thread_driven_from_a_second_and_played_on_a_third,
    a_finite_tail_never_reads_as_infinite,
    a_block_padded_with_nuls_loads_and_one_followed_by_anything_else_fails,
    a_release_reports_the_faults_no_tick_reported,
);

fn the_parameters_new_is_handed_are_the_ones_the_host_writes<S: Shell>() {
    let mut host = S::create();
    host.write(0.5);
    assert_eq!(host.activate(), Ok(()));
    assert_eq!(
        host.play(1.0),
        [6.0; 8],
        "the processor read 5 through the plugin's own parameter"
    );
}

fn a_plugin_created_on_one_thread_runs_on_the_thread_that_calls_it_first<S: Elsewhere>() {
    let mut host = S::create_elsewhere();
    host.write(0.5);
    assert_eq!(host.activate(), Ok(()), "the main thread is this one");
    assert_eq!(
        seen(|seen| seen.activations),
        1,
        "the plugin is called here"
    );
    assert_eq!(host.play(1.0), [6.0; 8]);
    host.deactivate();
    assert_eq!(seen(|seen| seen.deactivations), 1);
}

fn an_activation_while_active_rebuilds_and_recovers_a_panicked_processor<S: Shell>() {
    let mut host = S::create();
    host.write(0.5);
    assert_eq!(host.activate(), Ok(()));
    assert_eq!(host.play(1.0), [6.0; 8]);
    host.expect_faults(1);
    seen(|seen| seen.panic_in_process = true);
    assert_eq!(host.play(1.0), [0.0; 8], "the panicking block is silent");
    seen(|seen| seen.panic_in_process = false);
    assert_eq!(host.play(1.0), [0.0; 8], "and every block after it");

    assert_eq!(host.activate(), Ok(()), "again, while active");
    assert_eq!(seen(|seen| seen.activations), 2, "a fresh processor");
    assert_eq!(seen(|seen| seen.deactivations), 1, "the old one went back");
    assert_eq!(host.play(1.0), [6.0; 8], "playing again");
}

fn a_block_with_no_processor_is_silent<S: Shell>() {
    let mut host = S::create();
    host.write(0.5);
    assert_eq!(host.play(1.0), [0.0; 8], "never activated");
    assert_eq!(host.activate(), Ok(()));
    assert_eq!(host.play(1.0), [6.0; 8]);
    host.deactivate();
    assert_eq!(host.play(1.0), [0.0; 8], "deactivated");
}

fn a_reset_runs_before_the_next_block_unless_an_activation_replaced_the_processor<S: Shell>() {
    let mut host = S::create();
    assert_eq!(host.activate(), Ok(()));
    host.start();
    host.play(0.0);
    host.ask_reset();
    assert_eq!(seen(|seen| seen.resets), 0, "not until a block");
    host.play(0.0);
    host.play(0.0);
    assert_eq!(seen(|seen| seen.resets), 1, "once, before the next block");

    host.ask_reset();
    assert_eq!(host.activate(), Ok(()));
    host.play(0.0);
    assert_eq!(
        seen(|seen| seen.resets),
        1,
        "the new processor has nothing to clear"
    );
}

fn a_controller_write_while_processing_is_held_off_only_where_the_host_sends_it_again<S: Shell>() {
    let mut host = S::create();
    assert_eq!(host.activate(), Ok(()));
    host.start();
    host.write(0.25);
    let expected = if S::HOLDS_WRITES_WHILE_PROCESSING {
        0.0
    } else {
        0.25
    };
    assert_eq!(host.gain(), expected);
    host.deactivate();
    host.write(0.75);
    assert_eq!(host.gain(), 0.75, "deactivation ends processing");
}

fn a_fault_on_the_audio_path_is_reported_at_the_next_tick_once_per_instance_and_kind<S: Shell>() {
    let mut host = S::create();
    assert_eq!(host.activate(), Ok(()));
    written();
    let total = boundary_faults();
    host.expect_faults(2);
    seen(|seen| seen.panic_in_process = true);
    host.play(1.0);
    assert_eq!(
        written(),
        Vec::<String>::new(),
        "nothing written on the audio path"
    );
    host.tick();
    let lines = written();
    assert_eq!(lines.len(), 1, "{lines:?}");
    assert!(
        lines[0].starts_with("seam: Engine probe (instance ")
            && lines[0].contains("the processor panicked"),
        "{lines:?}"
    );
    assert_eq!(host.activate(), Ok(()), "recovered");
    host.play(1.0);
    host.tick();
    assert_eq!(
        written(),
        Vec::<String>::new(),
        "once per instance and kind"
    );
    seen(|seen| seen.panic_in_process = false);
    assert!(boundary_faults() >= total + 2, "counted for the process");
}

fn the_block_runs_marked_as_the_audio_thread_and_no_processor_drops_there<S: Shell>() {
    let mut host = S::create();
    assert_eq!(host.activate(), Ok(()));
    host.play(1.0);
    assert_eq!(
        seen(|seen| seen.marked_blocks),
        Some(cfg!(debug_assertions)),
        "marked in a debug build, which has the mark"
    );
    assert!(!audio_thread::is_here(), "and only for the block");
    host.expect_faults(1);
    seen(|seen| seen.panic_in_process = true);
    host.play(1.0);
    seen(|seen| seen.panic_in_process = false);
    assert_eq!(host.play(1.0), [0.0; 8], "silent, the processor kept");
    assert_eq!(
        host.activate(),
        Ok(()),
        "replacing the processor that panicked"
    );
    host.play(1.0);
    host.deactivate();
    assert_eq!(seen(|seen| (seen.drops, seen.drops_marked)), (2, 0));
}

fn created_on_one_thread_driven_from_a_second_and_played_on_a_third<S: Elsewhere>() {
    let mut host = S::create_elsewhere();
    host.write(0.5);
    assert_eq!(host.activate(), Ok(()));
    host.start();
    let (left, lines) = host.play_elsewhere(1.0, false);
    assert_eq!(left, [6.0; 8]);
    assert!(lines.is_empty(), "{lines:?}");
    host.tick();
    assert_eq!(written(), Vec::<String>::new(), "no fault anywhere");

    host.expect_faults(1);
    let (left, lines) = host.play_elsewhere(1.0, true);
    assert_eq!(left, [0.0; 8], "the panicking block is silent");
    assert_eq!(
        lines,
        Vec::<String>::new(),
        "nothing written on the audio thread"
    );
    host.tick();
    let lines = written();
    assert!(
        lines.len() == 1 && lines[0].contains("the processor panicked"),
        "reported from the main thread's tick: {lines:?}"
    );
}

fn a_finite_tail_never_reads_as_infinite<S: Shell>() {
    let mut host = S::create();
    for (tail, answer) in [
        (Tail::Frames(u32::MAX), u32::MAX - 1),
        (Tail::Frames(480), 480),
        (Tail::Infinite, u32::MAX),
        (Tail::None, 0),
    ] {
        seen(|seen| seen.tail = Some(tail));
        assert_eq!(host.activate(), Ok(()));
        assert_eq!(host.tail(), answer, "{tail:?}");
    }
}

fn a_block_padded_with_nuls_loads_and_one_followed_by_anything_else_fails<S: Shell>() {
    let mut host = S::create();
    host.write(0.5);
    let block = host.save();
    host.write(0.25);
    let mut padded = block.clone();
    padded.extend([0; 16]);
    assert_eq!(host.load(&padded), Ok(()), "NUL padding is trimmed");
    assert_eq!(host.gain(), 0.5);

    host.write(0.25);
    let mut followed = block.clone();
    followed.extend(br#"{"next": "chunk"}"#);
    assert!(
        host.load(&followed).is_err(),
        "the host is told the load failed"
    );
    assert!(host.load(b"\0\0").is_err(), "padding alone is no block");
    assert!(host.load(b"not a block").is_err());
    assert_eq!(host.gain(), 0.25, "and nothing changed");
}

fn a_release_reports_the_faults_no_tick_reported<S: Shell>() {
    let mut host = S::create();
    assert_eq!(host.activate(), Ok(()));
    host.expect_faults(1);
    seen(|seen| seen.panic_in_process = true);
    host.play(1.0);
    seen(|seen| seen.panic_in_process = false);
    written();
    drop(host);
    let lines = written();
    assert!(
        lines.len() == 1 && lines[0].contains("the processor panicked"),
        "reported as the instance went: {lines:?}"
    );
}

/// What a plugin here that keeps its parts hands the test, which runs it on
/// the test's own thread.
#[derive(Default)]
struct Handed {
    /// The number the plugin's participant holds.
    zoom: Rc<RefCell<Zoom>>,
    /// What the participant held when the plugin's idle last ran.
    zoom_at_idle: Option<f64>,
    history: Option<History>,
    /// A plain value the history records.
    level: Option<Value<f32>>,
    /// Learn's audio side, which a block would feed.
    learn: Option<LearnAudio>,
}

thread_local! {
    static HANDED: RefCell<Handed> = RefCell::default();
}

fn handed<R>(f: impl FnOnce(&mut Handed) -> R) -> R {
    HANDED.with(|handed| f(&mut handed.borrow_mut()))
}

/// A participant: a number saved under `zoom`, 1 by default.
#[derive(Debug)]
struct Zoom(f64);

impl Default for Zoom {
    fn default() -> Self {
        Zoom(1.0)
    }
}

impl Participant for Zoom {
    fn capture(&self) -> Section {
        Section::from(self.0)
    }

    fn apply(&mut self, section: Option<&Section>) -> Result<(), ApplyError> {
        self.0 = section.and_then(Section::as_f64).unwrap_or(1.0);
        Ok(())
    }
}

/// Registers a fresh [`Zoom`] with `parts`' session, handed to the test.
fn zoom(parts: &Parts) -> Registration {
    let zoom = Rc::new(RefCell::new(Zoom::default()));
    let registration = parts.session().register("zoom", &zoom).expect("a free key");
    handed(|handed| {
        *handed = Handed {
            zoom,
            ..Handed::default()
        }
    });
    registration
}

/// The host's saved block with `zoom` under the participant's key.
fn with_zoom(block: &[u8], zoom: f64) -> Vec<u8> {
    let mut block: Json = serde_json::from_slice(block).expect("JSON");
    block["extra"]["zoom"] = Json::from(zoom);
    serde_json::to_vec(&block).expect("JSON writes")
}

/// Keeps its parts and a participant, and notes in its idle what the
/// participant holds.
struct Restoring {
    _parts: Parts,
    _zoom: Registration,
}

impl Plugin for Restoring {
    const INFO: PluginInfo = Probe::INFO;
    type Params = Gain;
    type Processor = Adder;

    fn new(_: Gain, parts: Parts) -> Self {
        Restoring {
            _zoom: zoom(&parts),
            _parts: parts,
        }
    }

    fn activate(&mut self, _: &ProcessSetup) -> Adder {
        unreachable!("never activated")
    }

    fn idle(&mut self) {
        handed(|handed| handed.zoom_at_idle = Some(handed.zoom.borrow().0));
    }
}

/// Keeps its parts, with undo, learn and a participant, and writes no idle.
struct Idleless {
    _parts: Parts,
    _zoom: Registration,
}

impl Plugin for Idleless {
    const INFO: PluginInfo = PluginInfo {
        midi_input: true,
        ..Probe::INFO
    };
    type Params = Gain;
    type Processor = Adder;

    fn new(params: Gain, mut parts: Parts) -> Self {
        let history = parts.undo(Bound::default());
        let level = Value::new(0.0_f32, parts.set());
        history.enroll(&level, "Level").expect("a new value");
        parts.midi_learn(Config::default());
        parts
            .learn()
            .arm(params.gain.id(), Takes::CONTROLLER)
            .expect("a target");
        // As an activation would take it; this plugin is never activated.
        let learn = Driver::of(&parts).learn_audio().expect("learn is on");
        let registration = zoom(&parts);
        handed(|handed| {
            handed.history = Some(history);
            handed.level = Some(level);
            handed.learn = Some(learn);
        });
        Idleless {
            _parts: parts,
            _zoom: registration,
        }
    }

    fn activate(&mut self, _: &ProcessSetup) -> Adder {
        unreachable!("never activated")
    }
}

through_every_shell!(
    Restoring:
    seam_ticks_the_parts_before_the_plugin_s_idle,
);

through_every_shell!(
    Idleless:
    a_plugin_that_writes_no_idle_still_has_its_parts_ticked,
);

through_every_shell!(
    Probe:
    a_plugin_that_dropped_its_parts_is_ticked_with_nothing_reported,
);

fn seam_ticks_the_parts_before_the_plugin_s_idle<S: Shell>() {
    let mut host = S::create();
    host.tick();
    assert_eq!(handed(|handed| handed.zoom_at_idle), Some(1.0));
    let block = with_zoom(&host.save(), 2.0);
    assert_eq!(host.load(&block), Ok(()));
    host.tick();
    assert_eq!(
        handed(|handed| handed.zoom_at_idle),
        Some(2.0),
        "the idle after a load sees it applied"
    );
    host.expect_faults(0);
}

fn a_plugin_that_writes_no_idle_still_has_its_parts_ticked<S: Shell>() {
    let mut host = S::create();
    // An edit, and a controller learn hears, as a block would bring it.
    handed(|handed| {
        handed.level.as_ref().expect("handed").set(0.5);
        let voice = Voice::Midi1ControlChange {
            controller: 74,
            value: 64,
        };
        let mut events = EventList::with_capacity(1);
        events.push(
            ChannelVoice {
                group: 0,
                channel: 0,
                voice,
            }
            .event(0),
        );
        handed.learn.as_mut().expect("handed").process(&events);
    });
    host.tick();
    let history = handed(|handed| handed.history.clone()).expect("handed");
    assert_eq!(history.len(), 1, "the edit is one undo step");
    let block: Json = serde_json::from_slice(&host.save()).expect("JSON");
    assert_eq!(
        block["extra"][SESSION_KEY]["mappings"][0]["target"], "gain",
        "the learned mapping is saved"
    );

    let block = with_zoom(&host.save(), 2.0);
    assert_eq!(host.load(&block), Ok(()));
    host.tick();
    assert_eq!(handed(|handed| handed.zoom.borrow().0), 2.0, "restored");
    host.expect_faults(0);
}

fn a_plugin_that_dropped_its_parts_is_ticked_with_nothing_reported<S: Shell>() {
    let mut host = S::create();
    written();
    for _ in 0..3 {
        host.tick();
    }
    assert_eq!(written(), Vec::<String>::new());
    host.expect_faults(0);
}

/// Keeps its parts with learn on, so each activation takes learn's audio
/// side.
struct Learning {
    _parts: Parts,
    params: Gain,
}

impl Plugin for Learning {
    const INFO: PluginInfo = PluginInfo {
        midi_input: true,
        ..Probe::INFO
    };
    type Params = Gain;
    type Processor = Adder;

    fn new(params: Gain, mut parts: Parts) -> Self {
        parts.midi_learn(Config::default());
        Learning {
            _parts: parts,
            params,
        }
    }

    fn activate(&mut self, _: &ProcessSetup) -> Adder {
        Adder(self.params.clone())
    }
}

/// Keeps its parts with learn off.
struct Unlearned {
    _parts: Parts,
    params: Gain,
}

impl Plugin for Unlearned {
    const INFO: PluginInfo = Learning::INFO;
    type Params = Gain;
    type Processor = Adder;

    fn new(params: Gain, parts: Parts) -> Self {
        Unlearned {
            _parts: parts,
            params,
        }
    }

    fn activate(&mut self, _: &ProcessSetup) -> Adder {
        Adder(self.params.clone())
    }
}

through_every_shell!(
    Learning:
    each_activation_takes_learn_s_audio_side_and_drops_the_last_off_the_audio_thread,
);

through_every_shell!(
    Unlearned:
    an_activation_takes_no_audio_side_of_learn_while_learn_is_off,
);

through_every_shell!(
    Probe:
    an_activation_takes_no_audio_side_of_learn_from_parts_the_plugin_dropped,
);

fn each_activation_takes_learn_s_audio_side_and_drops_the_last_off_the_audio_thread<S: Shell>() {
    let mut host = S::create();
    assert_eq!(host.holds_learn_audio(), None, "no processor yet");
    assert_eq!(host.activate(), Ok(()));
    assert_eq!(host.holds_learn_audio(), Some(true));
    host.start();
    assert_eq!(host.play(1.0), [1.0; 8], "a block runs learn, then plays");
    // A rebuild takes a fresh one, and the last goes with its processor, on
    // this thread: dropped on the audio thread, it would panic.
    assert_eq!(host.activate(), Ok(()));
    assert_eq!(host.holds_learn_audio(), Some(true));
    assert_eq!(host.play(1.0), [1.0; 8]);
    host.deactivate();
    assert_eq!(host.holds_learn_audio(), None);
    assert_eq!(seen(|seen| (seen.drops, seen.drops_marked)), (2, 0));
    host.expect_faults(0);
}

fn an_activation_takes_no_audio_side_of_learn<S: Shell>() {
    let mut host = S::create();
    assert_eq!(host.activate(), Ok(()));
    assert_eq!(host.holds_learn_audio(), Some(false));
    assert_eq!(host.play(1.0), [1.0; 8]);
    host.expect_faults(0);
}

fn an_activation_takes_no_audio_side_of_learn_while_learn_is_off<S: Shell>() {
    an_activation_takes_no_audio_side_of_learn::<S>();
}

fn an_activation_takes_no_audio_side_of_learn_from_parts_the_plugin_dropped<S: Shell>() {
    an_activation_takes_no_audio_side_of_learn::<S>();
}

/// Version 1 of its schema renamed `level` to `gain`.
struct Renamed(Gain);

impl Plugin for Renamed {
    const INFO: PluginInfo = Probe::INFO;
    type Params = Gain;
    const SCHEMA_VERSION: u64 = 1;
    type Processor = Adder;

    fn new(params: Gain, _: Parts) -> Self {
        Renamed(params)
    }

    fn migrate(
        from: u64,
        block: &mut Map<String, Json>,
    ) -> Result<(), Box<dyn Error + Send + Sync>> {
        if from < 1 {
            let parameters = block
                .get_mut("parameters")
                .and_then(Json::as_object_mut)
                .ok_or("the parameters are not an object")?;
            if let Some(level) = parameters.remove("level") {
                parameters.entry("gain").or_insert(level);
            }
        }
        Ok(())
    }

    fn activate(&mut self, _: &ProcessSetup) -> Adder {
        Adder(self.0.clone())
    }
}

through_every_shell!(
    Renamed:
    the_plugin_s_migration_upgrades_an_older_block,
);

fn the_plugin_s_migration_upgrades_an_older_block<S: Shell>() {
    let mut host = S::create();
    let older = br#"{"seamStateVersion": 1, "parameters": {"level": 5}}"#;
    assert_eq!(host.load(older), Ok(()));
    assert_eq!(host.gain(), 0.5, "the renamed parameter took the value");
    let saved: Json = serde_json::from_slice(&host.save()).expect("JSON");
    assert_eq!(saved["pluginStateVersion"], 1, "saved at the new version");
    assert_eq!(saved["parameters"]["gain"], 5.0);
}
