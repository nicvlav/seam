//! The C interface through its table, as the JUCE adapter drives it.

use std::cell::{Cell, RefCell};
use std::ffi::c_void;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, LazyLock};

use seam_params::{
    AudioParams, Gesture, Kind, ParamDecl, ParamId, ParamSet, ProgramParams, UiParams, host_id,
};
use seam_session::Session;

use super::instance::{Callbacks, Instance};
use super::{
    ABI_VERSION, HostCall, KIND_BOOL, KIND_CHOICE, KIND_CONTINUOUS, PARAM_BYPASS, RESTART_LATENCY,
    RESTART_VALUES, STATUS_CONTINUE, STATUS_CONTINUE_IF_NOT_QUIET, STATUS_SLEEP, STATUS_TAIL,
    SeamHost, SeamInstance, SeamLayout, SeamParamInfo, SeamPlugin, SeamProcess, SeamStr,
    TAIL_INFINITE, TAIL_NONE, TestHost, header, juce_settings, plugin,
};
use crate::boundary::tests::Watched;
use crate::faults::{thread_faults, written};
use crate::{
    AuCodes, Block, Category, Feature, Layout, Plugin, PluginInfo, ProcessSetup, Processor,
    SavedState, Status, Tail,
};
use seam_testkit::sync::{Latch, PATIENCE, wait_until};

const PARAMS: &[ParamDecl] = &[
    ParamDecl::new(
        "gain",
        "Gain",
        Kind::continuous(-24.0, 24.0, 0.1, None),
        0.0,
    )
    .with_unit("dB"),
    ParamDecl::new("mode", "Mode", Kind::Choice(&["A", "B", "C"]), 1.0),
    // In a group, which hosts list after the top level.
    ParamDecl::new("bypass", "Bypass", Kind::Bool, 0.0)
        .with_group("Out")
        .as_bypass(),
];

const GAIN: u32 = 0;
const MODE: u32 = 1;
const BYPASS: u32 = 2;

/// What the probe was asked to do and what it saw, shared by the plugin and
/// its processor, which a test host runs on the test's own thread.
#[derive(Default)]
struct ProbeState {
    idles: u32,
    deactivations: u32,
    drops: u32,
    resets: u32,
    panic_in_process: bool,
    panic_in_activate: bool,
    panic_in_idle: bool,
    edit: Option<(&'static str, f32)>,
    sync: Option<(&'static str, f32)>,
    hold: bool,
    latency: u32,
    tail: Option<Tail>,
    status: Option<Status>,
    flushed: Option<bool>,
    blocks: Vec<usize>,
    setups: Vec<ProcessSetup>,
    /// Processors dropped, wherever they drop.
    processor_drops: Arc<AtomicU32>,
    panic_in_processor_drop: bool,
}

thread_local! {
    static PROBE: RefCell<ProbeState> = RefCell::default();
}

fn probe<R>(f: impl FnOnce(&mut ProbeState) -> R) -> R {
    PROBE.with(|state| f(&mut state.borrow_mut()))
}

struct Probe {
    saved: Arc<SavedState>,
    ui: UiParams,
    program: ProgramParams,
    held: Option<Gesture>,
}

impl Probe {
    fn id(&self, key: &str) -> ParamId {
        self.saved.set().id(key).expect("declared")
    }
}

struct ProbeProcessor {
    params: AudioParams,
    gain: ParamId,
    drops: Arc<AtomicU32>,
    panic_on_drop: bool,
}

impl Drop for ProbeProcessor {
    fn drop(&mut self) {
        self.drops.fetch_add(1, Ordering::Relaxed);
        assert!(!self.panic_on_drop, "a processor's drop's bug");
    }
}

impl Processor for ProbeProcessor {
    fn process(&mut self, block: Block<'_, '_>) -> Status {
        assert!(!probe(|state| state.panic_in_process), "a processor's bug");
        let tiny = std::hint::black_box(f32::MIN_POSITIVE);
        let product = std::hint::black_box(tiny * std::hint::black_box(0.5));
        probe(|state| {
            state.flushed = Some(product == 0.0);
            state.blocks.push(block.frames);
        });
        let add = self.params.value(self.gain);
        for channel in block.channels.iter_mut() {
            for sample in &mut channel[..block.frames] {
                *sample += add;
            }
        }
        probe(|state| state.status).unwrap_or(Status::Continue)
    }

    fn reset(&mut self) {
        probe(|state| state.resets += 1);
    }
}

impl Drop for Probe {
    fn drop(&mut self) {
        probe(|state| state.drops += 1);
    }
}

impl Plugin for Probe {
    const INFO: PluginInfo = PluginInfo {
        name: "Probe",
        vendor: "Seam",
        url: "https://seam.example",
        email: "probe@seam.example",
        description: "A \"probe\"",
        version: "1.2.3",
        id: "dev.seam.probe",
        vst3_class_id: [1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16],
        au_codes: AuCodes {
            subtype: *b"prob",
            manufacturer: *b"Seam",
        },
        category: Category::Effect,
        features: &[Feature::Equalizer, Feature::PitchShift],
        layouts: &[Layout::STEREO, Layout::MONO],
        midi_input: false,
        midi_output: false,
    };
    const PARAMS: &'static [ParamDecl] = PARAMS;
    type Processor = ProbeProcessor;

    fn new() -> Self {
        // Each test's probe starts afresh, even on a thread another used.
        probe(|state| *state = ProbeState::default());
        let set = ParamSet::from_decls(PARAMS.iter().copied()).expect("valid");
        let session = Session::new(&set);
        Probe {
            saved: Arc::new(SavedState::new(&set, &session)),
            ui: set.take_ui().expect("the first UI handle"),
            program: set.program(),
            held: None,
        }
    }

    fn saved(&self) -> Arc<SavedState> {
        Arc::clone(&self.saved)
    }

    fn activate(&mut self, setup: &ProcessSetup) -> ProbeProcessor {
        assert!(
            !probe(|state| state.panic_in_activate),
            "an activation's bug"
        );
        probe(|state| state.setups.push(*setup));
        ProbeProcessor {
            params: self.saved.set().audio(),
            gain: self.id("gain"),
            drops: probe(|state| Arc::clone(&state.processor_drops)),
            panic_on_drop: probe(|state| state.panic_in_processor_drop),
        }
    }

    fn deactivate(&mut self, processor: ProbeProcessor) {
        probe(|state| state.deactivations += 1);
        drop(processor);
    }

    fn latency(&self) -> u32 {
        probe(|state| state.latency)
    }

    fn tail(&self) -> Tail {
        probe(|state| state.tail).unwrap_or(Tail::Frames(100))
    }

    fn idle(&mut self) {
        probe(|state| state.idles += 1);
        assert!(!probe(|state| state.panic_in_idle), "an idle call's bug");
        if let Some((key, value)) = probe(|state| state.edit.take()) {
            assert!(self.ui.write(self.id(key), value));
        }
        if let Some((key, value)) = probe(|state| state.sync.take()) {
            self.program.sync(self.id(key), value);
        }
        if probe(|state| state.hold) && self.held.is_none() {
            let gesture = self.ui.begin(self.id("gain"));
            gesture.write(3.0);
            self.held = Some(gesture);
        }
    }
}

/// A host with the probe active at 48 kHz in stereo, `max_frames` per
/// block.
fn running(max_frames: u32) -> TestHost<Probe> {
    let host = TestHost::<Probe>::new();
    assert!(host.activate(48_000.0, max_frames, Layout::STEREO));
    host
}

/// An instance pointer that a test hands to another thread.
#[derive(Clone, Copy)]
struct Shared(*mut SeamInstance);

// SAFETY: the tests below hand it to one other thread at a time, to make
// calls the table allows from any thread or must refuse there.
unsafe impl Send for Shared {}

#[test]
fn the_table_lists_its_layouts_and_every_parameter() {
    let host = TestHost::<Probe>::new();
    assert_eq!(host.table().abi_version, ABI_VERSION);
    assert_eq!(host.layouts(), [Layout::STEREO, Layout::MONO]);
    let mut layout = SeamLayout {
        inputs: 7,
        outputs: 7,
    };
    // SAFETY: `layout` is valid for writing one `SeamLayout` during the call.
    let third = unsafe { (host.table().layout)(2, &mut layout) };
    assert!(!third, "no third");
    assert_eq!(host.parameter_count(), 3);
    let gain = host.parameter(GAIN).expect("listed");
    assert_eq!(
        (gain.key.as_str(), gain.name.as_str(), gain.unit.as_str()),
        ("gain", "Gain", "dB")
    );
    assert_eq!(gain.group, "");
    assert_eq!(gain.host_id, host_id("gain"));
    assert_eq!((gain.kind, gain.steps, gain.flags), (KIND_CONTINUOUS, 0, 0));
    assert_eq!(gain.default_normalized, 0.5);
    let mode = host.parameter(MODE).expect("listed");
    assert_eq!((mode.kind, mode.steps), (KIND_CHOICE, 3));
    assert_eq!(mode.default_normalized, 0.5);
    let bypass = host.parameter(BYPASS).expect("listed");
    assert_eq!(
        (bypass.kind, bypass.steps, bypass.flags),
        (KIND_BOOL, 2, PARAM_BYPASS)
    );
    assert_eq!(bypass.group, "Out");
    assert_eq!(host.parameter(3), None);
}

#[test]
fn values_and_their_text_go_through_the_declarations() {
    let host = TestHost::<Probe>::new();
    let decl = &PARAMS[0];
    assert_eq!(host.get(GAIN), 0.5);
    host.set(GAIN, 0.75);
    assert_eq!(host.get(GAIN), 0.75);
    host.set(GAIN, 0.501);
    assert_eq!(host.get(GAIN), 0.5, "snapped to the step");
    assert_eq!(host.get(9), 0.0, "an unknown index");
    host.set(9, 1.0);

    let full = decl.format_real(12.0);
    assert_eq!(host.text(GAIN, 0.75, 64), (full.clone(), full.len()));
    assert_eq!(host.text(MODE, 1.0, 64), ("C".to_owned(), 1));
    assert_eq!(
        host.text(9, 0.5, 64),
        (String::new(), 0),
        "an unknown index"
    );
    assert_eq!(
        host.text(GAIN, 0.75, 0),
        (String::new(), full.len()),
        "no room: the length alone"
    );
    // SAFETY: a call on the live instance with a null buffer, which it must
    // not write.
    let length =
        unsafe { (host.table().text)(host.instance(), GAIN, 0.75, std::ptr::null_mut(), 64) };
    assert_eq!(length, full.len(), "a size query");
    assert_eq!(
        host.text(GAIN, 0.75, 3),
        (full[..2].to_owned(), full.len()),
        "cut to fit, answering the whole length"
    );
    assert_eq!(host.parse(GAIN, "6"), Some(decl.kind.normalize(6.0)));
    assert_eq!(host.parse(GAIN, "loud"), None);
    assert_eq!(host.parse(MODE, "B"), Some(0.5));
    assert_eq!(host.parse(9, "6"), None, "an unknown index");
}

#[test]
fn text_is_cut_at_a_character() {
    const UNIT: &[ParamDecl] =
        &[ParamDecl::new("level", "Level", Kind::Choice(&["éé", "b"]), 0.0).with_unit("µ")];
    struct Accents(Arc<SavedState>);
    impl Plugin for Accents {
        const INFO: PluginInfo = Probe::INFO;
        const PARAMS: &'static [ParamDecl] = UNIT;
        type Processor = ProbeProcessor;
        fn new() -> Self {
            let set = ParamSet::from_decls([UNIT[0]]).expect("valid");
            Accents(Arc::new(SavedState::new(&set, &Session::new(&set))))
        }
        fn saved(&self) -> Arc<SavedState> {
            Arc::clone(&self.0)
        }
        fn activate(&mut self, _: &ProcessSetup) -> ProbeProcessor {
            unreachable!("never activated")
        }
    }
    let host = TestHost::<Accents>::new();
    assert_eq!(host.text(0, 0.0, 16), ("éé".to_owned(), 4));
    // "éé" is four bytes: room for three leaves one whole character.
    assert_eq!(host.text(0, 0.0, 4), ("é".to_owned(), 4));
    assert_eq!(host.parameter(0).expect("listed").unit, "µ");
}

#[test]
fn a_host_write_lands_from_any_thread() {
    let host = TestHost::<Probe>::new();
    let (table, shared) = (host.table(), Shared(host.instance()));
    std::thread::scope(|scope| {
        scope
            .spawn(move || {
                let faults = thread_faults();
                let instance = shared;
                // SAFETY: calls the header allows on any thread.
                unsafe {
                    (table.set)(instance.0, GAIN, 0.25);
                    assert_eq!((table.get)(instance.0, GAIN), 0.25);
                    assert_eq!((table.param_count)(instance.0), 3);
                    assert_eq!((table.latency)(instance.0), 0);
                }
                assert_eq!(thread_faults(), faults, "none refused");
            })
            .join()
            .expect("the writer");
    });
    assert_eq!(host.get(GAIN), 0.25);
}

#[test]
fn activation_builds_a_processor_for_the_setup_and_layout() {
    let host = TestHost::<Probe>::new();
    assert_eq!(host.tail(), 100, "answered from creation");
    assert!(
        !host.activate(48_000.0, 256, Layout::new(1, 2)),
        "not a listed layout"
    );
    assert!(!host.activate(f64::NAN, 256, Layout::STEREO), "no rate");
    assert!(!host.activate(-1.0, 256, Layout::STEREO), "a negative rate");
    assert!(probe(|state| state.setups.is_empty()));
    probe(|state| state.latency = 32);
    assert!(host.activate(44_100.0, 256, Layout::MONO));
    assert_eq!(host.latency(), 32, "read at activation");
    assert_eq!(
        probe(|state| state.setups.clone()),
        [ProcessSetup {
            sample_rate: 44_100.0,
            max_frames: 256,
            inputs: 1,
            outputs: 1,
            max_events: 0,
            max_out_events: 0,
        }]
    );
    // A second activation replaces the processor, as JUCE calls
    // `prepareToPlay` again on a new rate.
    assert!(host.activate(96_000.0, 128, Layout::STEREO));
    assert_eq!(probe(|state| state.deactivations), 1, "the first went back");
    host.idle();
    assert_eq!(host.calls(), [], "the latency was read with the activation");
    host.deactivate();
    host.deactivate();
    assert_eq!(probe(|state| state.deactivations), 2, "once");
}

#[test]
fn audio_is_processed_separately_in_place_and_in_pieces() {
    let host = running(16);
    host.set(GAIN, 0.75);
    let (left, right) = ([1.0f32; 40], [2.0f32; 40]);
    let (mut out_left, mut out_right, mut extra) = ([0.0f32; 40], [0.0f32; 40], [9.0f32; 40]);
    let status = host.process(
        &[&left, &right],
        &mut [&mut out_left, &mut out_right, &mut extra],
        40,
    );
    assert_eq!(status, STATUS_CONTINUE);
    assert_eq!((out_left, out_right), ([13.0; 40], [14.0; 40]));
    assert_eq!(extra, [0.0; 40], "a channel past the outputs is cleared");
    assert_eq!(
        probe(|state| state.blocks.clone()),
        [16, 16, 8],
        "longer than the activation"
    );
    let (mut left, mut right) = ([1.0f32; 4], [2.0f32; 4]);
    probe(|state| state.status = Some(Status::Sleep));
    let status = host.process_in_place(&mut [&mut left, &mut right], 4);
    assert_eq!(status, STATUS_SLEEP, "the processor's answer");
    assert_eq!((left, right), ([13.0; 4], [14.0; 4]));
    assert_eq!(
        probe(|state| state.flushed),
        Some(cfg!(all(
            not(miri),
            any(target_arch = "x86_64", target_arch = "aarch64")
        ))),
        "denormals flush inside"
    );
    let blocks = probe(|state| state.blocks.len());
    assert_eq!(host.process_in_place(&mut [&mut left, &mut right], 0), 0);
    assert_eq!(probe(|state| state.blocks.len()), blocks, "nothing to do");
}

#[test]
fn an_inactive_or_panicking_processor_plays_silence() {
    let host = TestHost::<Probe>::new();
    let mut left = [5.0f32; 8];
    host.process_in_place(&mut [&mut left], 8);
    assert_eq!(left, [0.0; 8], "silent before any activation");

    assert!(host.activate(48_000.0, 8, Layout::MONO));
    probe(|state| state.panic_in_process = true);
    host.expect_faults(1);
    let mut left = [5.0f32; 8];
    host.process_in_place(&mut [&mut left], 8);
    assert_eq!(left, [0.0; 8], "a panicking block is silenced");
    probe(|state| state.panic_in_process = false);
    let mut left = [5.0f32; 8];
    host.process_in_place(&mut [&mut left], 8);
    assert_eq!(left, [0.0; 8], "and so is every block after it");

    assert!(
        host.activate(48_000.0, 8, Layout::MONO),
        "a fresh processor"
    );
    let mut left = [5.0f32; 8];
    host.process_in_place(&mut [&mut left], 8);
    assert_eq!(left, [5.0; 8], "plays again");

    host.deactivate();
    let mut left = [5.0f32; 8];
    host.process_in_place(&mut [&mut left], 8);
    assert_eq!(left, [0.0; 8], "silent once deactivated");
}

#[test]
fn a_reset_reaches_the_processor_before_the_next_block() {
    let host = running(8);
    let mut left = [0.0f32; 8];
    host.process_in_place(&mut [&mut left], 8);
    assert_eq!(probe(|state| state.resets), 0);
    host.reset();
    assert_eq!(probe(|state| state.resets), 0, "not until a block");
    host.process_in_place(&mut [&mut left], 8);
    host.process_in_place(&mut [&mut left], 8);
    assert_eq!(probe(|state| state.resets), 1, "once");
}

#[test]
fn edits_syncs_and_latency_reach_the_host_inside_idle() {
    let host = TestHost::<Probe>::new();
    host.idle();
    assert_eq!(host.calls(), []);
    probe(|state| state.edit = Some(("gain", 6.0)));
    host.idle();
    assert_eq!(
        host.calls(),
        [
            HostCall::Begin(GAIN),
            HostCall::Perform(GAIN, PARAMS[0].kind.normalize(6.0)),
            HostCall::End(GAIN),
        ]
    );
    probe(|state| state.sync = Some(("mode", 0.0)));
    host.idle();
    assert_eq!(host.calls(), [HostCall::Restart(RESTART_VALUES)]);
    probe(|state| state.latency = 64);
    assert_eq!(host.latency(), 0, "what the main thread last read");
    host.idle();
    assert_eq!(host.calls(), [HostCall::Restart(RESTART_LATENCY)]);
    assert_eq!(host.latency(), 64);
    host.idle();
    assert_eq!(host.calls(), [], "told once");
    assert_eq!(probe(|state| state.idles), 5);

    // A gesture still open when the instance goes is ended for the host.
    probe(|state| state.hold = true);
    host.idle();
    assert_eq!(
        host.calls(),
        [
            HostCall::Begin(GAIN),
            HostCall::Perform(GAIN, PARAMS[0].kind.normalize(3.0))
        ]
    );
    assert_eq!(host.destroy(), [HostCall::End(GAIN)]);
}

#[test]
fn a_panic_in_activate_or_idle_is_answered() {
    let host = TestHost::<Probe>::new();
    probe(|state| state.panic_in_activate = true);
    host.expect_faults(1);
    assert!(!host.activate(48_000.0, 64, Layout::STEREO));
    probe(|state| state.panic_in_activate = false);
    assert!(
        host.activate(48_000.0, 64, Layout::STEREO),
        "the next one works"
    );
    probe(|state| state.panic_in_idle = true);
    host.expect_faults(1);
    host.idle();
    probe(|state| state.panic_in_idle = false);
    probe(|state| state.edit = Some(("gain", 6.0)));
    host.idle();
    assert_eq!(host.calls().len(), 3, "the tick runs again");
}

#[test]
fn a_main_thread_call_from_another_thread_is_refused() {
    let host = TestHost::<Probe>::new();
    // On the other thread; the host's check sees every thread's.
    host.expect_faults(3);
    let (table, shared) = (host.table(), Shared(host.instance()));
    std::thread::scope(|scope| {
        scope
            .spawn(move || {
                let instance = shared;
                let faults = thread_faults();
                // SAFETY: calls the instance must refuse on this thread
                // without touching its main-thread state.
                unsafe {
                    let stereo = SeamLayout {
                        inputs: 2,
                        outputs: 2,
                    };
                    assert!(!(table.activate)(instance.0, 48_000.0, 64, stereo));
                    (table.idle)(instance.0);
                    (table.deactivate)(instance.0);
                }
                assert_eq!(thread_faults(), faults + 3);
            })
            .join()
            .expect("the caller");
    });
    assert!(probe(|state| state.setups.is_empty()));
    assert_eq!(probe(|state| state.idles), 0);
}

/// Creates and activates an instance, destroys it on another thread, and
/// answers the faults counted there, the lines written there and the
/// instance, which leaks.
fn destroy_elsewhere(panic_in_processor_drop: bool) -> (u64, Vec<String>, Shared) {
    let table = plugin::<Probe>();
    let stereo = SeamLayout {
        inputs: 2,
        outputs: 2,
    };
    // SAFETY: `create` takes a null host as one with no callbacks, and the
    // activation is a call on the live instance it answered, on its main
    // thread.
    let instance = Shared(unsafe {
        let instance = (table.create)(std::ptr::null());
        probe(|state| state.panic_in_processor_drop = panic_in_processor_drop);
        assert!((table.activate)(instance, 48_000.0, 64, stereo));
        instance
    });
    let faults = std::thread::scope(|scope| {
        scope
            .spawn(move || {
                let instance = instance;
                let faults = thread_faults();
                written();
                // SAFETY: a destroy on the wrong thread, which leaks, with
                // no other call into the instance in flight.
                unsafe { (table.destroy)(instance.0) };
                (thread_faults() - faults, written())
            })
            .join()
            .expect("the releaser, the host not aborted")
    });
    let (faults, lines) = faults;
    (faults, lines, instance)
}

#[test]
fn a_destroy_on_another_thread_leaks_the_plugin_drops_the_processor_and_counts() {
    let (faults, lines, instance) = destroy_elsewhere(false);
    assert_eq!(faults, 1);
    assert!(
        lines.len() == 1 && lines[0].contains("released there and leaked"),
        "reported where it was destroyed: {lines:?}"
    );
    assert_eq!(
        probe(|state| (state.deactivations, state.drops)),
        (0, 0),
        "the plugin was never called or dropped off its thread"
    );
    assert_eq!(
        probe(|state| state.processor_drops.load(Ordering::Relaxed)),
        1,
        "the processor dropped there"
    );
    // SAFETY: the leaked instance is still whole, on its main thread; a
    // call after `destroy` is the host's mistake, which this survives.
    unsafe { (plugin::<Probe>().idle)(instance.0) };
    assert_eq!(probe(|state| state.idles), 0, "it ticks no more");
}

#[test]
fn a_processor_that_panics_as_it_drops_on_another_thread_is_caught_there() {
    let (faults, _, _) = destroy_elsewhere(true);
    assert_eq!(faults, 2, "the destroy, and the panic it caught");
    assert_eq!(
        probe(|state| state.processor_drops.load(Ordering::Relaxed)),
        1
    );
}

/// A host whose first callback destroys the instance from inside a tick.
struct Releaser {
    instance: Cell<*mut SeamInstance>,
}

unsafe extern "C" fn destroy_from_callback(context: *mut c_void, _index: u32) {
    // SAFETY: the context is the test's releaser.
    let releaser = unsafe { &*context.cast::<Releaser>() };
    let instance = releaser.instance.replace(std::ptr::null_mut());
    if !instance.is_null() {
        // SAFETY: the instance this test made, destroyed on its main thread
        // inside its own tick.
        unsafe { (plugin::<Probe>().destroy)(instance) };
    }
}

#[test]
fn a_destroy_inside_a_tick_leaks_it_deactivated() {
    let faults = thread_faults();
    let table = plugin::<Probe>();
    let releaser = Box::new(Releaser {
        instance: Cell::new(std::ptr::null_mut()),
    });
    let host = SeamHost {
        context: std::ptr::from_ref::<Releaser>(&releaser)
            .cast_mut()
            .cast::<c_void>(),
        begin_edit: Some(destroy_from_callback),
        ..SeamHost::NONE
    };
    // SAFETY: `host` is readable for `create`, its callbacks never unwind,
    // and its context, the releaser, outlives the instance; every later call
    // is on the instance `create` answered, on its main thread, with no
    // other call in flight.
    unsafe {
        let instance = (table.create)(&host);
        releaser.instance.set(instance);
        let stereo = SeamLayout {
            inputs: 2,
            outputs: 2,
        };
        assert!((table.activate)(instance, 48_000.0, 64, stereo));
        probe(|state| state.edit = Some(("gain", 6.0)));
        (table.idle)(instance);
    }
    assert!(releaser.instance.get().is_null(), "destroyed in the tick");
    assert_eq!(probe(|state| state.deactivations), 1, "deactivated");
    assert_eq!(probe(|state| state.drops), 0, "but left to leak");
    assert_eq!(thread_faults(), faults, "a host's right, not a fault");
}

#[test]
fn the_saved_block_round_trips_and_a_stranger_changes_nothing() {
    let host = TestHost::<Probe>::new();
    host.set(GAIN, 0.75);
    host.set(MODE, 1.0);
    let saved = host.save().expect("a block");
    let json: serde_json::Value = serde_json::from_slice(&saved).expect("JSON");
    assert_eq!(json["parameters"]["gain"], 12.0);
    assert_eq!(json["parameters"]["mode"], 2);
    host.set(GAIN, 0.0);
    assert!(host.load(&saved));
    assert_eq!(host.get(GAIN), 0.75);
    assert!(!host.load(b"not a block"), "not one");
    assert!(!host.load(b""), "nor is nothing");
    assert_eq!(host.get(GAIN), 0.75, "and neither changed anything");
}

#[test]
fn null_pointers_are_answered_not_followed() {
    let table = plugin::<Probe>();
    let null = std::ptr::null_mut::<SeamInstance>();
    let faults = thread_faults();
    let mut info = SeamParamInfo {
        key: SeamStr::new(""),
        name: SeamStr::new(""),
        unit: SeamStr::new(""),
        group: SeamStr::new(""),
        host_id: 0,
        kind: 0,
        steps: 0,
        flags: 0,
        default_normalized: 0.0,
    };
    let block = SeamProcess {
        inputs: std::ptr::null(),
        outputs: std::ptr::null(),
        input_count: 2,
        output_count: 2,
        frames: 64,
    };
    let stereo = SeamLayout {
        inputs: 2,
        outputs: 2,
    };
    // SAFETY: every call with a null instance, which each must refuse.
    unsafe {
        assert_eq!((table.param_count)(null), 0);
        assert!(!(table.param_info)(null, 0, &mut info));
        assert_eq!((table.get)(null, 0), 0.0);
        (table.set)(null, 0, 1.0);
        assert_eq!((table.text)(null, 0, 0.5, std::ptr::null_mut(), 8), 0);
        assert!(!(table.activate)(null, 48_000.0, 64, stereo));
        (table.deactivate)(null);
        (table.idle)(null);
        assert_eq!((table.latency)(null), 0);
        assert!(!(table.save)(null, std::ptr::null_mut(), None));
        assert!(!(table.load)(null, std::ptr::null(), 0));
        (table.reset)(null);
        assert_eq!((table.process)(null, &block), STATUS_CONTINUE);
        (table.destroy)(null);
        assert!(!(table.layout)(0, std::ptr::null_mut()));
    }
    let host = running(64);
    // SAFETY: a live instance with null arguments, which it must refuse.
    unsafe {
        assert!(!(table.param_info)(
            host.instance(),
            0,
            std::ptr::null_mut()
        ));
        assert!(!(table.save)(host.instance(), std::ptr::null_mut(), None));
        assert!(!(table.load)(host.instance(), std::ptr::null(), 4));
        let text = SeamStr {
            data: std::ptr::null(),
            length: 3,
        };
        let mut normalized = 0.0;
        assert!(!(table.parse)(host.instance(), GAIN, text, &mut normalized));
        assert_eq!((table.process)(host.instance(), std::ptr::null()), 0);
        // A block with no channel lists plays nothing and writes nowhere.
        assert_eq!((table.process)(host.instance(), &block), STATUS_CONTINUE);
    }
    assert_eq!(thread_faults(), faults);
}

#[test]
fn the_header_declares_every_struct_and_asserts_its_layout() {
    let text = header();
    assert!(text.starts_with("// seam.h"));
    assert!(text.contains("#define SEAM_ABI_VERSION 1u"));
    assert!(text.contains("#define SEAM_TAIL_INFINITE 4294967295u"));
    assert!(text.contains("const SeamPlugin* seam_plugin(void);"));
    for item in super::abi::structs() {
        assert!(
            text.contains(&format!("typedef struct {} {{", item.name)),
            "{}",
            item.name
        );
        assert!(text.contains(&format!(
            "SEAM_LAYOUT(sizeof({}) == {});",
            item.name, item.size
        )));
        let mut last = None;
        for field in &item.fields {
            assert!(
                last.is_none_or(|offset| field.offset > offset),
                "{}.{} in order",
                item.name,
                field.name
            );
            last = Some(field.offset);
            assert!(
                field.declaration.contains(field.name),
                "{}: {}",
                field.name,
                field.declaration
            );
        }
    }
    assert!(
        text.contains("    uint32_t (*process)(SeamInstance* instance, const SeamProcess* block);")
    );
    assert_eq!(
        std::mem::size_of::<super::SeamPlugin>(),
        8 * 20,
        "a version, padded, then 19 calls"
    );
}

#[test]
fn the_header_s_wait_is_the_loan_wait_s_own() {
    // The header states the wait in words a C user can read, so no duration
    // it names may drift from the constant the engine waits for, however it
    // is written.
    let wait = crate::engine::LOAN_WAIT.as_secs_f64() * 1000.0;
    let named = durations_in_ms(&header());
    assert!(!named.is_empty(), "the header names the wait");
    assert!(
        named.iter().all(|ms| (ms - wait).abs() < 1e-9),
        "the header names {named:?} ms; the wait is {wait} ms"
    );
}

#[test]
fn a_duration_is_read_however_it_is_written() {
    for written in [
        "up to 50 ms,",
        "50ms",
        "50-ms",
        "50 msec",
        "50 milliseconds",
        "fifty milliseconds",
        "fifty ms",
        "0.05 s",
        "0.05 seconds",
        "up to 50\n    // ms, and",
    ] {
        assert_eq!(durations_in_ms(written), [50.0], "{written:?}");
    }
    assert_eq!(durations_in_ms("twenty-five ms"), [25.0]);
    assert_eq!(durations_in_ms("one hundred milliseconds"), [100.0]);
    assert_eq!(durations_in_ms("1 second, then 2 ms"), [1000.0, 2.0]);
    for none in [
        "about 60 times a second",
        "C11's layout",
        "uint32_t frames",
        "SEAM_MAX_FRAMES",
    ] {
        assert_eq!(durations_in_ms(none), Vec::<f64>::new(), "{none:?}");
    }
}

/// Every duration `text` names, in milliseconds. A duration is a number,
/// in digits or in words, then a unit of milliseconds or seconds, apart
/// from it by nothing, spaces or a hyphen: "50 ms", "50ms", "fifty
/// milliseconds", "0.05 s". A comment's line breaks count as spaces.
fn durations_in_ms(text: &str) -> Vec<f64> {
    const ONES: [&str; 20] = [
        "zero",
        "one",
        "two",
        "three",
        "four",
        "five",
        "six",
        "seven",
        "eight",
        "nine",
        "ten",
        "eleven",
        "twelve",
        "thirteen",
        "fourteen",
        "fifteen",
        "sixteen",
        "seventeen",
        "eighteen",
        "nineteen",
    ];
    const TENS: [&str; 8] = [
        "twenty", "thirty", "forty", "fifty", "sixty", "seventy", "eighty", "ninety",
    ];
    let prose = text
        .lines()
        .map(|line| line.trim().trim_start_matches("//").trim())
        .collect::<Vec<_>>()
        .join(" ");
    // Each word or number, and whether only spaces or a hyphen part it from
    // the one before.
    let mut tokens: Vec<(String, bool)> = Vec::new();
    let (mut token, mut joined) = (String::new(), true);
    let chars: Vec<char> = prose.chars().collect();
    for (at, &c) in chars.iter().enumerate() {
        let digit = |c: char| c.is_ascii_digit();
        let in_number = digit(c)
            || (c == '.' && token.chars().all(digit) && !token.is_empty())
                && chars.get(at + 1).copied().is_some_and(digit);
        let continues = !token.is_empty()
            && if token.starts_with(digit) {
                in_number
            } else {
                c.is_alphabetic()
            };
        if continues {
            token.push(c);
            continue;
        }
        if !token.is_empty() {
            tokens.push((std::mem::take(&mut token).to_lowercase(), joined));
            joined = true;
        }
        if in_number || c.is_alphabetic() {
            token.push(c);
        } else if c != ' ' && c != '-' {
            joined = false;
        }
    }
    if !token.is_empty() {
        tokens.push((token.to_lowercase(), joined));
    }

    let mut found = Vec::new();
    let mut at = 0;
    while at < tokens.len() {
        // A number in digits, or the longest run of number words.
        let (mut value, mut next) = (None, at);
        if let Ok(number) = tokens[at].0.parse::<f64>() {
            (value, next) = (Some(number), at + 1);
        } else {
            let mut spelled = 0.0;
            while let Some((word, _)) = tokens.get(next).filter(|(_, joined)| next == at || *joined)
            {
                if let Some(one) = ONES.iter().position(|one| one == word) {
                    spelled += one as f64;
                } else if let Some(ten) = TENS.iter().position(|ten| ten == word) {
                    spelled += 10.0 * (ten + 2) as f64;
                } else if word == "hundred" && next > at {
                    spelled *= 100.0;
                } else {
                    break;
                }
                next += 1;
            }
            if next > at {
                value = Some(spelled);
            }
        }
        let scale = tokens
            .get(next)
            .filter(|(_, joined)| *joined)
            .and_then(|(unit, _)| match unit.as_str() {
                "ms" | "msec" | "msecs" | "millisecond" | "milliseconds" => Some(1.0),
                "s" | "sec" | "secs" | "second" | "seconds" => Some(1000.0),
                _ => None,
            });
        if let (Some(value), Some(scale)) = (value, scale) {
            found.push(value * scale);
        }
        at = next.max(at + 1);
    }
    found
}

#[test]
fn the_juce_settings_name_the_plugin() {
    let text = juce_settings::<Probe>("the probe");
    assert!(text.contains("Generated by `cargo xtask gen` from the probe."));
    for line in [
        "set(SEAM_PLUGIN_NAME \"Probe\")",
        "set(SEAM_PLUGIN_DESCRIPTION \"A \\\"probe\\\"\")",
        "set(SEAM_PLUGIN_VERSION \"1.2.3\")",
        "set(SEAM_PLUGIN_ID \"dev.seam.probe\")",
        "set(SEAM_PLUGIN_CODE \"prob\")",
        "set(SEAM_PLUGIN_MANUFACTURER_CODE \"Seam\")",
        "set(SEAM_PLUGIN_IS_SYNTH FALSE)",
        "set(SEAM_PLUGIN_MIDI_INPUT FALSE)",
        "set(SEAM_PLUGIN_VST3_CATEGORIES \"Fx\" \"EQ\" \"Pitch Shift\")",
    ] {
        assert!(text.contains(line), "{line}\n{text}");
    }
}

#[test]
fn the_header_spells_each_type_as_c_does() {
    use super::abi::CType;
    assert_eq!(
        <*const *const f32>::declare("inputs", &[]),
        "const float* const* inputs"
    );
    assert_eq!(
        <*const *mut f32>::declare("outputs", &[]),
        "float* const* outputs"
    );
    assert_eq!(<*mut SeamInstance>::declare("", &[]), "SeamInstance*");
    assert_eq!(
        <unsafe extern "C" fn(*mut SeamInstance, u32, f64)>::declare(
            "set",
            &["instance", "index", "normalized"]
        ),
        "void (*set)(SeamInstance* instance, uint32_t index, double normalized)"
    );
    assert_eq!(
        <unsafe extern "C" fn() -> u32>::declare("count", &[]),
        "uint32_t (*count)(void)"
    );
    assert_eq!(
        <Option<super::WriteFn>>::declare("write", &[]),
        "void (*write)(void*, const uint8_t*, size_t)"
    );
    let text = header();
    assert!(text.contains(
        "    bool (*save)(SeamInstance* instance, void* context, void (*write)(void*, const uint8_t*, size_t));"
    ));
    assert!(text.contains("    const char* data;"));
}

#[test]
fn deactivation_takes_the_processor_back_and_reads_the_answers() {
    let host = running(8);
    probe(|state| {
        state.latency = 32;
        state.tail = Some(Tail::Infinite);
    });
    host.deactivate();
    assert_eq!(probe(|state| state.deactivations), 1);
    assert_eq!(
        (host.latency(), host.tail()),
        (32, TAIL_INFINITE),
        "read at deactivation, no tick needed"
    );
    probe(|state| state.tail = Some(Tail::None));
    host.idle();
    assert_eq!(host.tail(), TAIL_NONE);
    assert_eq!(
        host.calls(),
        [HostCall::Restart(RESTART_LATENCY)],
        "the host last read the activation's"
    );
    probe(|state| state.tail = Some(Tail::Frames(7)));
    host.idle();
    assert_eq!(host.tail(), 7);
    assert!(host.activate(48_000.0, 8, Layout::STEREO));
    assert_eq!(
        probe(|state| state.deactivations),
        1,
        "nothing to hand back"
    );
    assert_eq!(host.destroy(), [], "no gesture was open");
    assert_eq!(
        probe(|state| state.deactivations),
        2,
        "destroy hands the processor back through the plugin"
    );
}

#[test]
fn an_activation_drops_a_reset_asked_of_the_processor_it_replaces() {
    let host = running(8);
    host.reset();
    assert!(
        host.activate(48_000.0, 8, Layout::STEREO),
        "a fresh processor"
    );
    let mut left = [0.0f32; 8];
    host.process_in_place(&mut [&mut left], 8);
    assert_eq!(probe(|state| state.resets), 0, "nothing to clear");
}

#[test]
fn every_status_reaches_the_host() {
    let host = running(8);
    let mut left = [0.0f32; 8];
    for (status, code) in [
        (Status::Continue, STATUS_CONTINUE),
        (Status::ContinueIfNotQuiet, STATUS_CONTINUE_IF_NOT_QUIET),
        (Status::Tail, STATUS_TAIL),
        (Status::Sleep, STATUS_SLEEP),
    ] {
        probe(|state| state.status = Some(status));
        assert_eq!(
            host.process_in_place(&mut [&mut left], 8),
            code,
            "{status:?}"
        );
    }
}

/// The probe as an instrument that takes MIDI and sends none: no input, or
/// a mono one.
struct Instrument(Probe);

impl Plugin for Instrument {
    const INFO: PluginInfo = PluginInfo {
        category: Category::Instrument,
        features: &[Feature::Synthesizer],
        layouts: &[Layout::new(0, 2), Layout::new(1, 2)],
        midi_input: true,
        midi_output: false,
        ..Probe::INFO
    };
    const PARAMS: &'static [ParamDecl] = PARAMS;
    type Processor = ProbeProcessor;

    fn new() -> Self {
        Instrument(Probe::new())
    }

    fn saved(&self) -> Arc<SavedState> {
        self.0.saved()
    }

    fn activate(&mut self, setup: &ProcessSetup) -> ProbeProcessor {
        self.0.activate(setup)
    }
}

#[test]
fn an_instrument_plays_with_no_input_or_a_narrower_one() {
    let host = TestHost::<Instrument>::new();
    assert_eq!(host.layouts(), [Layout::new(0, 2), Layout::new(1, 2)]);
    assert!(
        !host.activate(48_000.0, 8, Layout::STEREO),
        "not a listed layout"
    );
    assert!(host.activate(48_000.0, 8, Layout::new(0, 2)));
    let setup = probe(|state| state.setups[0]);
    assert_eq!((setup.inputs, setup.outputs), (0, 2));
    assert_eq!(setup.max_events, 2048, "it takes MIDI");
    host.set(GAIN, 0.75);
    let (mut left, mut right) = ([9.0f32; 8], [9.0f32; 8]);
    host.process(&[], &mut [&mut left, &mut right], 8);
    assert_eq!((left, right), ([12.0; 8], [12.0; 8]), "from silence");

    assert!(host.activate(48_000.0, 8, Layout::new(1, 2)));
    let input = [1.0f32; 8];
    host.process(&[&input], &mut [&mut left, &mut right], 8);
    assert_eq!(
        (left, right),
        ([13.0; 8], [12.0; 8]),
        "the second channel starts silent"
    );
    let settings = juce_settings::<Instrument>("the instrument");
    for line in [
        "set(SEAM_PLUGIN_IS_SYNTH TRUE)",
        "set(SEAM_PLUGIN_MIDI_INPUT TRUE)",
        "set(SEAM_PLUGIN_MIDI_OUTPUT FALSE)",
        "set(SEAM_PLUGIN_VST3_CATEGORIES \"Instrument\" \"Synth\")",
    ] {
        assert!(settings.contains(line), "{line}\n{settings}");
    }
}

/// A host whose callbacks record their names and can re-enter the instance:
/// tick it again from `begin_edit`, or destroy it from either edit
/// callback.
struct Reentrant {
    instance: Cell<*mut SeamInstance>,
    calls: RefCell<Vec<&'static str>>,
    nest_on_begin: bool,
    destroy_on: &'static str,
}

impl Reentrant {
    /// The test's host from a callback's context.
    ///
    /// # Safety
    ///
    /// `context` points to a `Reentrant` that outlives `'a`, shared only
    /// through `&` references.
    unsafe fn from<'a>(context: *mut c_void) -> &'a Reentrant {
        // SAFETY: the caller's contract.
        unsafe { &*context.cast::<Reentrant>() }
    }

    fn heard(&self, call: &'static str) {
        self.calls.borrow_mut().push(call);
        if self.destroy_on == call {
            let instance = self.instance.replace(std::ptr::null_mut());
            if !instance.is_null() {
                // SAFETY: the instance this test made, destroyed on its main
                // thread from inside its own tick.
                unsafe { (plugin::<Probe>().destroy)(instance) };
            }
        }
    }

    fn host(&self) -> SeamHost {
        SeamHost {
            context: std::ptr::from_ref::<Reentrant>(self)
                .cast_mut()
                .cast::<c_void>(),
            begin_edit: Some(reentrant_begin),
            perform_edit: Some(reentrant_perform),
            end_edit: Some(reentrant_end),
            restart: Some(reentrant_restart),
        }
    }
}

unsafe extern "C" fn reentrant_begin(context: *mut c_void, _index: u32) {
    // SAFETY: the context is the test's `Reentrant`.
    let host = unsafe { Reentrant::from(context) };
    if host.nest_on_begin {
        let instance = host.instance.get();
        // SAFETY: the live instance, ticked again on its main thread from
        // inside its own tick, as a host's nested message loop would.
        unsafe { (plugin::<Probe>().idle)(instance) };
    }
    host.heard("begin");
}

unsafe extern "C" fn reentrant_perform(context: *mut c_void, _index: u32, _normalized: f64) {
    // SAFETY: as above.
    unsafe { Reentrant::from(context) }.heard("perform");
}

unsafe extern "C" fn reentrant_end(context: *mut c_void, _index: u32) {
    // SAFETY: as above.
    unsafe { Reentrant::from(context) }.heard("end");
}

unsafe extern "C" fn reentrant_restart(context: *mut c_void, _flags: u32) {
    // SAFETY: as above.
    unsafe { Reentrant::from(context) }.heard("restart");
}

/// Creates and activates the probe with `host`'s callbacks, queues an edit
/// and a latency change, and ticks it once.
fn tick_once_with(host: &Reentrant) {
    let table = plugin::<Probe>();
    let callbacks = host.host();
    // SAFETY: `callbacks` is readable for `create`, its callbacks never
    // unwind, and its context, the test's host, outlives the instance; every
    // later call is on the instance `create` answered, on its main thread.
    unsafe {
        let instance = (table.create)(&callbacks);
        host.instance.set(instance);
        let stereo = SeamLayout {
            inputs: 2,
            outputs: 2,
        };
        assert!((table.activate)(instance, 48_000.0, 64, stereo));
        probe(|state| {
            state.edit = Some(("gain", 6.0));
            state.latency = 16;
        });
        (table.idle)(instance);
    }
}

#[test]
fn a_destroy_inside_a_tick_ends_the_ticks_calls_to_the_host() {
    let faults = thread_faults();
    let host = Box::new(Reentrant {
        instance: Cell::new(std::ptr::null_mut()),
        calls: RefCell::new(Vec::new()),
        nest_on_begin: false,
        destroy_on: "begin",
    });
    tick_once_with(&host);
    assert!(host.instance.get().is_null(), "destroyed in the tick");
    assert_eq!(
        *host.calls.borrow(),
        ["begin"],
        "no edit or restart after the destroy, whose host may be gone"
    );
    assert_eq!(probe(|state| state.deactivations), 1, "deactivated");
    assert_eq!(probe(|state| state.drops), 0, "but left to leak");
    assert_eq!(thread_faults(), faults);
}

#[test]
fn a_tick_nested_inside_a_tick_does_nothing() {
    let faults = thread_faults();
    let host = Box::new(Reentrant {
        instance: Cell::new(std::ptr::null_mut()),
        calls: RefCell::new(Vec::new()),
        nest_on_begin: true,
        destroy_on: "perform",
    });
    tick_once_with(&host);
    assert_eq!(probe(|state| state.idles), 1, "the nested tick did nothing");
    assert!(host.instance.get().is_null(), "destroyed in the outer tick");
    assert_eq!(
        probe(|state| state.drops),
        0,
        "left to leak, since the outer tick was still running"
    );
    assert_eq!(*host.calls.borrow(), ["begin", "perform"]);
    assert_eq!(thread_faults(), faults);
}

#[test]
fn the_juce_settings_refuse_keys_whose_juce_ids_collide() {
    const COLLIDING: &[ParamDecl] = &[
        ParamDecl::new("Aa", PARAMS[0].name, PARAMS[0].kind, PARAMS[0].default),
        ParamDecl::new("BB", PARAMS[0].name, PARAMS[0].kind, PARAMS[0].default),
    ];
    struct Colliding;
    impl Plugin for Colliding {
        const INFO: PluginInfo = Probe::INFO;
        const PARAMS: &'static [ParamDecl] = COLLIDING;
        type Processor = ProbeProcessor;
        fn new() -> Self {
            unreachable!("never created")
        }
        fn saved(&self) -> Arc<SavedState> {
            unreachable!("never created")
        }
        fn activate(&mut self, _: &ProcessSetup) -> ProbeProcessor {
            unreachable!("never created")
        }
    }
    let refused = std::panic::catch_unwind(|| juce_settings::<Colliding>("colliding keys"));
    let message = refused.expect_err("refused");
    let message = message
        .downcast_ref::<String>()
        .map(String::as_str)
        .unwrap_or_default();
    assert!(message.contains("\"BB\""), "{message}");
}

#[test]
fn the_juce_settings_refuse_a_midi_controllers_id_only_for_midi_input() {
    // A key whose JUCE id is the first of the MIDI controller parameters
    // JUCE adds for a plugin that takes MIDI input.
    const CONTROLLER: &[ParamDecl] = &[ParamDecl::new(
        "cc.bfgmpdl",
        PARAMS[0].name,
        PARAMS[0].kind,
        PARAMS[0].default,
    )];
    struct Keyed<const MIDI_INPUT: bool>;
    impl<const MIDI_INPUT: bool> Plugin for Keyed<MIDI_INPUT> {
        const INFO: PluginInfo = PluginInfo {
            midi_input: MIDI_INPUT,
            ..Probe::INFO
        };
        const PARAMS: &'static [ParamDecl] = CONTROLLER;
        type Processor = ProbeProcessor;
        fn new() -> Self {
            unreachable!("never created")
        }
        fn saved(&self) -> Arc<SavedState> {
            unreachable!("never created")
        }
        fn activate(&mut self, _: &ProcessSetup) -> ProbeProcessor {
            unreachable!("never created")
        }
    }
    let settings = juce_settings::<Keyed<false>>("no MIDI input");
    assert!(settings.contains("set(SEAM_PLUGIN_MIDI_INPUT FALSE)"));
    let refused = std::panic::catch_unwind(|| juce_settings::<Keyed<true>>("MIDI input"));
    let message = refused.expect_err("refused");
    let message = message
        .downcast_ref::<String>()
        .map(String::as_str)
        .unwrap_or_default();
    assert!(message.contains("MIDI controller"), "{message}");
}

/// A plugin with no state but its saved block, whose processor passes audio
/// through; `Send`, so its instance is lent to any thread.
struct Mover(Arc<SavedState>);

struct Through;

impl Processor for Through {
    fn process(&mut self, _: Block<'_, '_>) -> Status {
        Status::Continue
    }
}

impl Plugin for Mover {
    const INFO: PluginInfo = Probe::INFO;
    const PARAMS: &'static [ParamDecl] = PARAMS;
    type Processor = Through;

    fn new() -> Self {
        let set = ParamSet::from_decls(PARAMS.iter().copied()).expect("valid");
        Mover(Arc::new(SavedState::new(&set, &Session::new(&set))))
    }

    fn saved(&self) -> Arc<SavedState> {
        Arc::clone(&self.0)
    }

    fn activate(&mut self, _: &ProcessSetup) -> Through {
        Through
    }
}

/// The same plugin holding an `Rc`, so not `Send`: it may not leave the
/// thread that made it.
struct Fixed(Mover, std::marker::PhantomData<std::rc::Rc<()>>);

impl Plugin for Fixed {
    const INFO: PluginInfo = Probe::INFO;
    const PARAMS: &'static [ParamDecl] = PARAMS;
    type Processor = Through;

    fn new() -> Self {
        Fixed(Mover::new(), std::marker::PhantomData)
    }

    fn saved(&self) -> Arc<SavedState> {
        self.0.saved()
    }

    fn activate(&mut self, setup: &ProcessSetup) -> Through {
        self.0.activate(setup)
    }
}

/// Creates an instance from `table` on another thread, then activates and
/// destroys it on this one, and answers whether the activation took and the
/// faults counted here.
fn created_elsewhere_then_activated(table: &'static SeamPlugin) -> (bool, u64) {
    let instance = std::thread::scope(|scope| {
        // SAFETY: `create` takes a null host as one with no callbacks.
        scope
            .spawn(|| Shared(unsafe { (table.create)(std::ptr::null()) }))
            .join()
            .expect("created on another thread")
    });
    let faults = thread_faults();
    let stereo = SeamLayout {
        inputs: 2,
        outputs: 2,
    };
    // SAFETY: calls on the live instance `create` answered, with no other
    // call in flight; `destroy` is its last.
    let activated = unsafe {
        let activated = (table.activate)(instance.0, 48_000.0, 64, stereo);
        (table.destroy)(instance.0);
        activated
    };
    (activated, thread_faults() - faults)
}

#[test]
fn the_exported_table_lends_a_send_plugin_to_any_thread_and_keeps_any_other_where_it_was_made() {
    use super::BoundAtCreation as _;
    assert_eq!(
        created_elsewhere_then_activated(<super::Table<Mover>>::TABLE),
        (true, 0),
        "taken on any thread"
    );
    assert_eq!(
        created_elsewhere_then_activated(<super::Table<Fixed>>::TABLE),
        (false, 2),
        "bound where it was made: the activation and the destroy are refused here"
    );
}

#[test]
fn an_activation_with_no_block_size_takes_the_fallbacks() {
    let host = TestHost::<Probe>::new();
    assert!(host.activate(96_000.0, 0, Layout::STEREO));
    let setup = probe(|state| state.setups[0]);
    assert_eq!((setup.sample_rate, setup.max_frames), (96_000.0, 1024));
    assert!(
        host.activate(48_000.0, super::MAX_FRAMES, Layout::STEREO),
        "at the limit"
    );
}

#[test]
fn an_activation_past_the_block_size_limit_builds_for_the_limit() {
    let host = TestHost::<Probe>::new();
    for promised in [super::MAX_FRAMES + 1, u32::MAX] {
        assert!(
            host.activate(48_000.0, promised, Layout::STEREO),
            "{promised} frames"
        );
        let setup = probe(|state| state.setups.last().copied()).expect("built");
        assert_eq!(
            setup.max_frames,
            crate::MAX_FRAMES,
            "{promised} frames: longer blocks play in pieces"
        );
    }
}

#[test]
fn a_moving_plugin_takes_main_thread_calls_from_any_thread_however_many_ticks_ran() {
    // As JUCE's wrappers may call prepareToPlay off the message thread,
    // before the first tick or long after it.
    let host = TestHost::<Mover>::created_elsewhere();
    let (table, shared) = (host.table(), Shared(host.instance()));
    let elsewhere = || {
        std::thread::scope(|scope| {
            scope
                .spawn(move || {
                    let instance = shared;
                    let stereo = SeamLayout {
                        inputs: 2,
                        outputs: 2,
                    };
                    // SAFETY: main-thread calls on the live instance, with no
                    // other call into it in flight.
                    unsafe {
                        (table.idle)(instance.0);
                        (table.activate)(instance.0, 48_000.0, 64, stereo)
                    }
                })
                .join()
                .expect("the other thread")
        })
    };
    assert!(elsewhere(), "taken before the first tick here");
    for _ in 0..100 {
        host.idle();
    }
    assert!(elsewhere(), "and after a hundred");
    assert!(host.activate(48_000.0, 64, Layout::STEREO), "and here");
    assert_eq!(host.faults(), 0);
}

/// A call that holds the plugin until a test lets it go: `inside` opens once
/// it has the plugin, and `release` lets it finish.
struct Gates {
    inside: Latch,
    release: Latch,
}

impl Gates {
    fn new() -> Self {
        Gates {
            inside: Latch::new(1),
            release: Latch::new(1),
        }
    }
}

/// [`HeldActivation`]'s gates, and its ticks.
static ACTIVATION: LazyLock<Gates> = LazyLock::new(Gates::new);
static HELD_ACTIVATION_IDLES: AtomicU32 = AtomicU32::new(0);

/// A `Send` plugin whose activation holds the plugin until a test lets it
/// go.
struct HeldActivation(Mover);

impl Plugin for HeldActivation {
    const INFO: PluginInfo = Probe::INFO;
    const PARAMS: &'static [ParamDecl] = PARAMS;
    type Processor = Through;

    fn new() -> Self {
        HeldActivation(Mover::new())
    }

    fn saved(&self) -> Arc<SavedState> {
        self.0.saved()
    }

    fn activate(&mut self, setup: &ProcessSetup) -> Through {
        ACTIVATION.inside.count_down();
        ACTIVATION.release.wait();
        self.0.activate(setup)
    }

    fn idle(&mut self) {
        HELD_ACTIVATION_IDLES.fetch_add(1, Ordering::Relaxed);
    }
}

/// The gates of each [`HeldTick`], by its number: one per test.
static TICKS: [LazyLock<Gates>; 3] = [
    LazyLock::new(Gates::new),
    LazyLock::new(Gates::new),
    LazyLock::new(Gates::new),
];
/// How many of each [`HeldTick`] dropped.
static HELD_TICK_DROPS: [AtomicU32; 3] = [const { AtomicU32::new(0) }; 3];
/// How each [`HeldTick`] that [`watched`] makes waits for a loan, by its
/// number: up to [`PATIENCE`], counting each call that begins to wait.
static WAITS: [Watched; 3] = [const { Watched::new(PATIENCE) }; 3];

/// A table whose instances of [`HeldTick`] are bound to no thread, and wait
/// for a loan as `WAITS[GATE]` says. A test then meets a call while it
/// waits. The wait is long, since three threads must run before it runs
/// out, which the engine's own bound does not leave a loaded machine room
/// for.
fn watched<const GATE: usize>() -> &'static SeamPlugin {
    const { &super::table::<HeldTick<GATE>>(create_watched::<GATE>) }
}

/// [`watched`]'s `create`.
///
/// # Safety
///
/// As [`SeamPlugin::create`] and `seam.h`'s pointer rules say.
unsafe extern "C" fn create_watched<const GATE: usize>(host: *const SeamHost) -> *mut SeamInstance {
    // SAFETY: the header's contract: `host` is null or valid for reading
    // one `SeamHost` during this call, which copies it at once.
    let host = unsafe { host.as_ref() }.copied().unwrap_or(SeamHost::NONE);
    let instance = Instance::<HeldTick<GATE>>::new_unbound(Some(Callbacks(host)), &WAITS[GATE]);
    Box::into_raw(instance).cast()
}

/// A `Send` plugin whose tick holds the plugin until a test lets it go.
struct HeldTick<const GATE: usize>(Mover);

impl<const GATE: usize> Drop for HeldTick<GATE> {
    fn drop(&mut self) {
        HELD_TICK_DROPS[GATE].fetch_add(1, Ordering::Relaxed);
    }
}

impl<const GATE: usize> Plugin for HeldTick<GATE> {
    const INFO: PluginInfo = Probe::INFO;
    const PARAMS: &'static [ParamDecl] = PARAMS;
    type Processor = Through;

    fn new() -> Self {
        HeldTick(Mover::new())
    }

    fn saved(&self) -> Arc<SavedState> {
        self.0.saved()
    }

    fn activate(&mut self, setup: &ProcessSetup) -> Through {
        self.0.activate(setup)
    }

    fn idle(&mut self) {
        TICKS[GATE].inside.count_down();
        TICKS[GATE].release.wait();
    }
}

/// A main-thread activation of the instance behind `shared`, as a host's
/// own thread makes it.
fn activate_on(table: &'static SeamPlugin, shared: Shared) -> bool {
    let stereo = SeamLayout {
        inputs: 2,
        outputs: 2,
    };
    // SAFETY: a main-thread call on the live instance; the tests that make
    // it overlap it with another call on purpose.
    unsafe { (table.activate)(shared.0, 48_000.0, 64, stereo) }
}

#[test]
fn a_tick_while_an_activation_runs_elsewhere_skips_without_a_fault() {
    // JUCE's VST3 wrapper calls prepareToPlay from `setActive` on the host's
    // thread, holding no lock, while the adapter's timer ticks on the
    // message thread.
    let host = TestHost::<HeldActivation>::created_elsewhere();
    let (table, shared) = (host.table(), Shared(host.instance()));
    written();
    let activated = std::thread::scope(|scope| {
        let activation = scope.spawn(move || activate_on(table, shared));
        ACTIVATION.inside.wait();
        host.idle();
        ACTIVATION.release.count_down();
        activation.join().expect("the host's thread")
    });
    assert!(activated, "the activation ran whole");
    assert_eq!(
        HELD_ACTIVATION_IDLES.load(Ordering::Relaxed),
        0,
        "the tick skipped"
    );
    assert_eq!(host.faults(), 0, "and counted nothing");
    host.idle();
    assert_eq!(
        HELD_ACTIVATION_IDLES.load(Ordering::Relaxed),
        1,
        "the next tick runs"
    );
    assert_eq!(written(), Vec::<String>::new(), "and reports nothing");
}

/// Activates `host`'s instance on another thread once its tick, on this
/// one, has the plugin, and answers whether the activation took. The tick
/// lets go once the activation has its answer or, for an instance that
/// waits as `watched` says, once it waits for the plugin.
fn activated_during_a_tick<const GATE: usize>(
    host: &TestHost<HeldTick<GATE>>,
    watched: Option<&Watched>,
) -> bool {
    let (table, shared) = (host.table(), Shared(host.instance()));
    let returned = &AtomicBool::new(false);
    std::thread::scope(|scope| {
        let activation = scope.spawn(move || {
            TICKS[GATE].inside.wait();
            let activated = activate_on(table, shared);
            returned.store(true, Ordering::Release);
            activated
        });
        scope.spawn(move || {
            wait_until("the activation to wait or answer", || {
                returned.load(Ordering::Acquire)
                    || watched.is_some_and(|watched| watched.waits() > 0)
            });
            TICKS[GATE].release.count_down();
        });
        host.idle();
        activation.join().expect("the host's thread")
    })
}

#[test]
fn an_activation_while_a_tick_runs_elsewhere_waits_and_takes_it() {
    let host = TestHost::<HeldTick<0>>::create(watched::<0>(), true);
    assert!(
        activated_during_a_tick(&host, Some(&WAITS[0])),
        "it waited for the tick, then took the plugin"
    );
    assert_eq!(host.faults(), 0);
}

#[test]
fn a_main_thread_call_held_off_past_the_wait_is_refused_and_counted() {
    // A host holding a lock of its own across both threads would deadlock
    // here but for the bound: the tick lets go only once the activation
    // has given up.
    let host = TestHost::<HeldTick<1>>::created_elsewhere();
    host.expect_faults(1);
    assert!(!activated_during_a_tick(&host, None), "refused");
    assert_eq!(host.faults(), 1, "and counted");
    written();
    host.idle();
    let lines = written();
    assert!(
        lines.len() == 1 && lines[0].contains("waited too long for a call on another thread"),
        "reported as a wait that ran out, not as a wrong thread: {lines:?}"
    );
}

#[test]
fn a_destroy_while_another_thread_ticks_leaks_the_instance_without_waiting() {
    // A host that breaks the header's rule that no other call is in flight
    // during `destroy`. The tick goes on using the instance after it lets
    // the plugin go, until it returns, so the destroy may not wait for it
    // and then free the instance under it.
    let table = watched::<2>();
    // SAFETY: `create` takes a null host as one with no callbacks.
    let shared = Shared(unsafe { (table.create)(std::ptr::null()) });
    let destroyed = &AtomicBool::new(false);
    let (faults, lines) = std::thread::scope(|scope| {
        scope.spawn(move || {
            let instance = shared;
            // SAFETY: a tick on the live instance, which the destroy below
            // overlaps on purpose.
            unsafe { (table.idle)(instance.0) };
        });
        let destroy = scope.spawn(move || {
            let instance = shared;
            TICKS[2].inside.wait();
            // SAFETY: the live instance, destroyed while another thread's
            // tick holds it: a host's breach the destroy must survive.
            unsafe { (table.destroy)(instance.0) };
            destroyed.store(true, Ordering::Release);
            (thread_faults(), written())
        });
        // A destroy that waited for the tick would take the plugin as the
        // tick lets it go, so the tick lets go once the destroy waits.
        wait_until("the destroy to wait or return", || {
            destroyed.load(Ordering::Acquire) || WAITS[2].waits() > 0
        });
        TICKS[2].release.count_down();
        destroy.join().expect("the destroying thread")
    });
    assert_eq!(faults, 1, "the destroy was refused, and counted");
    assert!(
        lines.len() == 1 && lines[0].contains("released while one held it and leaked"),
        "and reported where it was made: {lines:?}"
    );
    assert_eq!(
        HELD_TICK_DROPS[2].load(Ordering::Relaxed),
        0,
        "the plugin never dropped: the instance leaked"
    );
    assert_eq!(WAITS[2].waits(), 0, "the destroy did not wait");
}

/// [`Syncer`]'s plugin calls and drops.
static SYNCER_IDLES: AtomicU32 = AtomicU32::new(0);
static SYNCER_DEACTIVATIONS: AtomicU32 = AtomicU32::new(0);
static SYNCER_DROPS: AtomicU32 = AtomicU32::new(0);

/// A `Send` plugin whose tick syncs the gain, so the host's `restart`
/// callback runs inside it.
struct Syncer(Mover, ProgramParams);

impl Drop for Syncer {
    fn drop(&mut self) {
        SYNCER_DROPS.fetch_add(1, Ordering::Relaxed);
    }
}

impl Plugin for Syncer {
    const INFO: PluginInfo = Probe::INFO;
    const PARAMS: &'static [ParamDecl] = PARAMS;
    type Processor = Through;

    fn new() -> Self {
        let mover = Mover::new();
        let program = mover.0.set().program();
        Syncer(mover, program)
    }

    fn saved(&self) -> Arc<SavedState> {
        self.0.saved()
    }

    fn activate(&mut self, setup: &ProcessSetup) -> Through {
        self.0.activate(setup)
    }

    fn deactivate(&mut self, _: Through) {
        SYNCER_DEACTIVATIONS.fetch_add(1, Ordering::Relaxed);
    }

    fn idle(&mut self) {
        let count = SYNCER_IDLES.fetch_add(1, Ordering::Relaxed);
        let gain = self.0.0.set().id("gain").expect("declared");
        let _ = self.1.sync(gain, count as f32 + 1.0);
    }
}

/// The instance [`tick_then_destroy_inside`] destroys from inside its tick.
static INSIDE: std::sync::atomic::AtomicPtr<SeamInstance> =
    std::sync::atomic::AtomicPtr::new(std::ptr::null_mut());

/// The host's `restart` callback: ticks the instance again, as a nested
/// message loop would, then destroys it.
unsafe extern "C" fn tick_then_destroy_inside(_context: *mut c_void, _flags: u32) {
    let instance = INSIDE.swap(std::ptr::null_mut(), Ordering::Relaxed);
    if !instance.is_null() {
        let table = super::plugin_moving::<Syncer>();
        // SAFETY: the instance this test made, called from inside its own
        // tick on the thread that ticks it, as the header lets a callback.
        unsafe {
            (table.idle)(instance);
            (table.destroy)(instance);
        }
    }
}

#[test]
fn a_moving_plugin_called_from_inside_its_own_tick_shares_that_tick_s_loan() {
    let faults = thread_faults();
    let table = super::plugin_moving::<Syncer>();
    let host = SeamHost {
        restart: Some(tick_then_destroy_inside),
        ..SeamHost::NONE
    };
    // SAFETY: `host` is readable for `create`, and its callback never
    // unwinds; every later call is on the instance `create` answered, with
    // no other call in flight.
    unsafe {
        let instance = (table.create)(&host);
        let stereo = SeamLayout {
            inputs: 2,
            outputs: 2,
        };
        assert!((table.activate)(instance, 48_000.0, 64, stereo));
        INSIDE.store(instance, Ordering::Relaxed);
        (table.idle)(instance);
    }
    assert!(
        INSIDE.load(Ordering::Relaxed).is_null(),
        "destroyed in the tick"
    );
    assert_eq!(
        SYNCER_IDLES.load(Ordering::Relaxed),
        1,
        "the nested tick did nothing"
    );
    assert_eq!(
        SYNCER_DEACTIVATIONS.load(Ordering::Relaxed),
        1,
        "deactivated"
    );
    assert_eq!(SYNCER_DROPS.load(Ordering::Relaxed), 0, "but left to leak");
    assert_eq!(thread_faults(), faults, "a host's right, not a fault");
}

// Only the VST3 shell starts a timer, so only a build with both has one
// to refuse.
#[cfg(feature = "vst3")]
#[test]
fn an_instance_bound_to_no_thread_starts_no_timer() {
    let table = super::plugin_moving::<Mover>();
    // SAFETY: `create` takes a null host as one with no callbacks.
    let pointer = unsafe { (table.create)(std::ptr::null()) };
    // SAFETY: the live instance `create` answered, used only before the
    // `destroy` below.
    let instance = unsafe { super::instance::<Mover>(pointer) }.expect("created");
    let faults = instance.faults();
    instance.start_timer();
    assert!(
        instance.timer_handle().is_none(),
        "no timer: no thread owns it"
    );
    assert_eq!(instance.faults(), faults + 1, "refused and counted");
    // SAFETY: the same instance, with no other call into it in flight.
    unsafe { (table.destroy)(pointer) };
}
