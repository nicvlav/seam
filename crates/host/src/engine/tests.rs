//! The engine's rules, each tested through every shell with one body.

use std::cell::RefCell;
use std::sync::Arc;

#[cfg(feature = "vst3")]
use seam_params::host_id;
use seam_params::{AudioParams, Kind, ParamDecl, ParamId, ParamSet, audio_thread};
use seam_session::Session;

use crate::faults::written;
use crate::{
    AuCodes, Block, Category, Layout, Plugin, PluginInfo, ProcessSetup, Processor, SavedState,
    Status, Tail, boundary_faults,
};

const PARAMS: &[ParamDecl] = &[ParamDecl::new(
    "gain",
    "Gain",
    Kind::continuous(0.0, 10.0, 0.0, None),
    0.0,
)];

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

/// A plugin whose processor adds its gain parameter to every sample.
struct Probe(Arc<SavedState>);

struct Adder {
    params: AudioParams,
    gain: ParamId,
}

impl Processor for Adder {
    fn process(&mut self, block: Block<'_, '_>) -> Status {
        let marked = audio_thread::is_here();
        seen(|seen| seen.marked_blocks = Some(seen.marked_blocks.unwrap_or(true) && marked));
        assert!(!seen(|seen| seen.panic_in_process), "a processor's bug");
        let add = self.params.value(self.gain);
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
    const PARAMS: &'static [ParamDecl] = PARAMS;
    type Processor = Adder;

    fn new() -> Self {
        seen(|seen| *seen = Seen::default());
        let set = ParamSet::from_decls([PARAMS[0]]).expect("valid");
        Probe(Arc::new(SavedState::new(&set, &Session::new(&set))))
    }

    fn saved(&self) -> Arc<SavedState> {
        Arc::clone(&self.0)
    }

    fn activate(&mut self, _: &ProcessSetup) -> Adder {
        seen(|seen| seen.activations += 1);
        Adder {
            params: self.0.set().audio(),
            gain: self.0.set().id("gain").expect("declared"),
        }
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
    fn create() -> Self;
    /// Creates the instance on another thread; this one calls it first.
    fn create_elsewhere() -> Self;
    /// Activates in stereo at 48 kHz for blocks of up to 64 frames.
    fn activate(&mut self) -> bool;
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
    /// The host's load of a block, and whether it says the load worked.
    fn load(&mut self, block: &[u8]) -> bool;
    fn expect_faults(&self, count: u64);
}

#[cfg(feature = "vst3")]
impl Shell for crate::vst3::TestHost<Probe> {
    const HOLDS_WRITES_WHILE_PROCESSING: bool = true;

    fn create() -> Self {
        Self::new()
    }

    fn create_elsewhere() -> Self {
        Self::created_elsewhere()
    }

    fn activate(&mut self) -> bool {
        // Refused while active, as the SDK has it; the setup stays.
        self.setup(48_000.0, 64);
        crate::vst3::TestHost::activate(self)
    }

    fn deactivate(&mut self) {
        assert!(crate::vst3::TestHost::deactivate(self));
    }

    fn start(&mut self) {
        assert!(crate::vst3::TestHost::start(self));
    }

    fn ask_reset(&mut self) {
        assert!(self.stop() && crate::vst3::TestHost::start(self));
    }

    fn play(&mut self, input: f32) -> [f32; 8] {
        let (mut left, mut right) = ([input; 8], [input; 8]);
        assert!(self.process_in_place(&mut [&mut left, &mut right], 8, &[]));
        left
    }

    fn play_elsewhere(&mut self, input: f32, panic: bool) -> ([f32; 8], Vec<String>) {
        let mut side = self.audio_side();
        on_another_thread(
            move || {
                let (mut left, mut right) = ([input; 8], [input; 8]);
                assert!(side.process_in_place(&mut [&mut left, &mut right], 8, &[]));
                left
            },
            panic,
        )
    }

    fn write(&mut self, normalized: f64) {
        assert!(self.set_normalized(host_id("gain"), normalized));
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

    fn load(&mut self, block: &[u8]) -> bool {
        crate::vst3::TestHost::load(self, block)
    }

    fn expect_faults(&self, count: u64) {
        crate::vst3::TestHost::expect_faults(self, count);
    }
}

#[cfg(feature = "cabi")]
impl Shell for crate::cabi::TestHost<Probe> {
    const HOLDS_WRITES_WHILE_PROCESSING: bool = false;

    fn create() -> Self {
        Self::new()
    }

    fn create_elsewhere() -> Self {
        Self::created_elsewhere()
    }

    fn activate(&mut self) -> bool {
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

    fn load(&mut self, block: &[u8]) -> bool {
        crate::cabi::TestHost::load(self, block)
    }

    fn expect_faults(&self, count: u64) {
        crate::cabi::TestHost::expect_faults(self, count);
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

/// Runs a shared body through every shell built.
macro_rules! through_every_shell {
    ($($body:ident),* $(,)?) => {
        $(
            mod $body {
                #[cfg(feature = "vst3")]
                #[test]
                fn vst3() {
                    super::$body::<crate::vst3::TestHost<super::Probe>>();
                }

                #[cfg(feature = "cabi")]
                #[test]
                fn cabi() {
                    super::$body::<crate::cabi::TestHost<super::Probe>>();
                }
            }
        )*
    };
}

through_every_shell!(
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

fn a_plugin_created_on_one_thread_runs_on_the_thread_that_calls_it_first<S: Shell>() {
    let mut host = S::create_elsewhere();
    host.write(0.5);
    assert!(host.activate(), "the main thread is this one");
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
    assert!(host.activate());
    assert_eq!(host.play(1.0), [6.0; 8]);
    host.expect_faults(1);
    seen(|seen| seen.panic_in_process = true);
    assert_eq!(host.play(1.0), [0.0; 8], "the panicking block is silent");
    seen(|seen| seen.panic_in_process = false);
    assert_eq!(host.play(1.0), [0.0; 8], "and every block after it");

    assert!(host.activate(), "again, while active");
    assert_eq!(seen(|seen| seen.activations), 2, "a fresh processor");
    assert_eq!(seen(|seen| seen.deactivations), 1, "the old one went back");
    assert_eq!(host.play(1.0), [6.0; 8], "playing again");
}

fn a_block_with_no_processor_is_silent<S: Shell>() {
    let mut host = S::create();
    host.write(0.5);
    assert_eq!(host.play(1.0), [0.0; 8], "never activated");
    assert!(host.activate());
    assert_eq!(host.play(1.0), [6.0; 8]);
    host.deactivate();
    assert_eq!(host.play(1.0), [0.0; 8], "deactivated");
}

fn a_reset_runs_before_the_next_block_unless_an_activation_replaced_the_processor<S: Shell>() {
    let mut host = S::create();
    assert!(host.activate());
    host.start();
    host.play(0.0);
    host.ask_reset();
    assert_eq!(seen(|seen| seen.resets), 0, "not until a block");
    host.play(0.0);
    host.play(0.0);
    assert_eq!(seen(|seen| seen.resets), 1, "once, before the next block");

    host.ask_reset();
    assert!(host.activate());
    host.play(0.0);
    assert_eq!(
        seen(|seen| seen.resets),
        1,
        "the new processor has nothing to clear"
    );
}

fn a_controller_write_while_processing_is_held_off_only_where_the_host_sends_it_again<S: Shell>() {
    let mut host = S::create();
    assert!(host.activate());
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
    assert!(host.activate());
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
    assert!(host.activate(), "recovered");
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
    assert!(host.activate());
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
    assert!(host.activate(), "replacing the processor that panicked");
    host.play(1.0);
    host.deactivate();
    assert_eq!(seen(|seen| (seen.drops, seen.drops_marked)), (2, 0));
}

fn created_on_one_thread_driven_from_a_second_and_played_on_a_third<S: Shell>() {
    let mut host = S::create_elsewhere();
    host.write(0.5);
    assert!(host.activate());
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
        assert!(host.activate());
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
    assert!(host.load(&padded), "NUL padding is trimmed");
    assert_eq!(host.gain(), 0.5);

    host.write(0.25);
    let mut followed = block.clone();
    followed.extend(br#"{"next": "chunk"}"#);
    assert!(!host.load(&followed), "the host is told the load failed");
    assert!(!host.load(b"\0\0"), "padding alone is no block");
    assert!(!host.load(b"not a block"));
    assert_eq!(host.gain(), 0.25, "and nothing changed");
}

fn a_release_reports_the_faults_no_tick_reported<S: Shell>() {
    let mut host = S::create();
    assert!(host.activate());
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
