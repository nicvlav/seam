//! The exporter core: one instance of a plugin as every exporter drives it.
//!
//! Each exporter is a thin shell over an [`Engine`], translating its host's
//! calls into the engine's. The engine holds the plugin's life: activation,
//! the block, the tick, what the host is told, and teardown. The rules it
//! keeps are on [`Engine`]; a shell adds only its format's shape.
//!
//! The engine is boxed and never moves, so a timer's tick can hold a pointer
//! to it; a shell frees it only when [`Engine::release`] says it may.

mod automation;
mod process;
pub(crate) mod timer;

#[cfg(all(test, any(feature = "vst3", feature = "cabi")))]
mod tests;

use std::cell::{Cell, RefCell};
use std::num::NonZeroUsize;
#[cfg(feature = "vst3")]
use std::ptr::NonNull;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::Duration;

#[cfg(any(feature = "vst3", feature = "cabi"))]
use seam_params::ParamId;
use seam_params::{AudioParams, HostEvent, HostEventDrain, ParamSet};

use automation::Automation;
pub(crate) use automation::Points;
#[cfg(all(test, feature = "vst3"))]
pub(crate) use automation::{MAX_SKIPPED_QUEUES, Point};
#[cfg(any(feature = "vst3", feature = "cabi"))]
pub(crate) use process::Outcome;
use process::{Audio, PendingReset, install};
use timer::{Handle, Timer, abandon};

#[cfg(feature = "cabi")]
use crate::boundary::Patience;
use crate::boundary::{AudioSlot, Lent, MainThread, Refused};
use crate::faults::{Fault, Faults};
use crate::{Layout, MAX_FRAMES, Plugin, SavedState, Tail};

/// What `tail` answers for a plugin whose output falls silent with its
/// input; VST3's `kNoTail` and the C interface's `SEAM_TAIL_NONE`.
pub(crate) const TAIL_NONE: u32 = 0;
/// What `tail` answers for a plugin that may never fall silent; VST3's
/// `kInfiniteTail` and the C interface's `SEAM_TAIL_INFINITE`.
pub(crate) const TAIL_INFINITE: u32 = u32::MAX;

/// The most MIDI events one block carries each way, for a plugin that takes
/// or sends MIDI.
const MAX_EVENTS: usize = 2048;

/// How long a main-thread call waits for a plugin bound to no thread that
/// another thread's call has on loan, before it is refused.
///
/// A tick and a release never wait (the threading rule on [`Engine`]). The
/// 50 ms wait is three frames of the 60 Hz tick, far longer than a tick
/// takes, so a call that meets one waits its turn and goes on. The bound
/// keeps a host that holds a lock of its own across both threads from
/// deadlocking. Past it the waiting call is refused, the host's thread goes
/// on, and the call that has the plugin can finish.
///
/// `SeamPlugin`'s doc, and so `seam.h`, states it for C users in
/// milliseconds; a test keeps the two equal.
///
/// Open: these calls are never the audio thread's by contract. But JUCE's
/// AAX wrapper calls prepareToPlay on the render thread when a longer block
/// arrives. An activation that meets a tick then waits there, on the audio
/// thread, for as long as the tick runs.
#[cfg_attr(
    not(feature = "cabi"),
    expect(
        dead_code,
        reason = "only the C interface makes an engine bound to no thread"
    )
)]
pub(crate) const LOAN_WAIT: Duration = Duration::from_millis(50);

/// The last instance's number, for naming an instance in its reports.
static INSTANCES: AtomicU64 = AtomicU64::new(0);

/// How a host will run the processor: what an activation is built for.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Setup {
    pub(crate) sample_rate: f64,
    /// Never 0, so a block always plays in pieces of at least a frame.
    pub(crate) max_frames: NonZeroUsize,
}

impl Setup {
    /// What an activation that was never given a setup is built for.
    pub(crate) const FALLBACK: Setup = Setup {
        sample_rate: 44_100.0,
        max_frames: NonZeroUsize::new(1024).unwrap(),
    };

    /// A setup of `sample_rate` and `max_frames`, or `None` when the rate is
    /// not a positive finite number.
    ///
    /// A block size past [`MAX_FRAMES`] takes `MAX_FRAMES`, and one of 0,
    /// which says nothing, takes the fallback's. Either way the rate is kept,
    /// and a block longer than the setup's plays in pieces.
    pub(crate) fn new(sample_rate: f64, max_frames: usize) -> Option<Setup> {
        let rate = sample_rate > 0.0 && sample_rate.is_finite();
        rate.then(|| Setup {
            sample_rate,
            max_frames: NonZeroUsize::new(max_frames.min(MAX_FRAMES))
                .unwrap_or(Setup::FALLBACK.max_frames),
        })
    }
}

/// What the host is asked to re-read.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct Restart {
    /// Every parameter's value: a sync changed values without a gesture.
    pub(crate) values: bool,
    /// The latency.
    pub(crate) latency: bool,
}

/// A shell's link to its host, on the main thread: how the plugin's edits
/// and restarts reach it.
///
/// The engine takes a clone for each call, so no borrow of its own is held
/// while the host runs and calls back in.
pub(crate) trait HostLink: Clone + 'static {
    /// Whether the host reads the latency as an activation returns, so an
    /// activation's latency counts as told. A host that may have asked
    /// before activating is told of a change at the next tick instead.
    const ACTIVATION_TELLS_LATENCY: bool;

    /// Whether the host sends its automation as queues of points with each
    /// block, which the engine then has room for.
    const QUEUES: bool;

    /// Tells the host of one edit.
    fn tell(&self, set: &ParamSet, event: HostEvent);

    /// Asks the host to re-read what `restart` names.
    fn restart(&self, restart: Restart);
}

/// What a shell does with the engine after [`Engine::release`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[must_use]
pub(crate) enum Release {
    /// Nothing reaches the engine any more: the shell frees it, here.
    Free,
    /// A tick, or the plugin on its own thread, may still reach it: the
    /// shell leaks it.
    Leak,
}

/// What the host may read on any thread. The main thread keeps it up to
/// date: at creation, at each activation and deactivation, at each tick,
/// and when a shell arranges the bus layout.
#[derive(Debug)]
struct Answers {
    /// The main bus layout: inputs in the high half, outputs in the low.
    layout: AtomicU64,
    latency: AtomicU32,
    /// As `tail` answers it.
    tail: AtomicU32,
}

impl Answers {
    fn new(layout: Layout, plugin: &impl Plugin) -> Self {
        let answers = Answers {
            layout: AtomicU64::new(0),
            latency: AtomicU32::new(0),
            tail: AtomicU32::new(TAIL_NONE),
        };
        answers.set_layout(layout);
        answers.read(plugin);
        answers
    }

    #[cfg(feature = "vst3")]
    fn layout(&self) -> Layout {
        let packed = self.layout.load(Ordering::Acquire);
        Layout::new(
            (packed >> 32) as usize,
            (packed & u64::from(u32::MAX)) as usize,
        )
    }

    fn set_layout(&self, layout: Layout) {
        let half = |channels: usize| u64::from(u32::try_from(channels).unwrap_or(u32::MAX));
        self.layout.store(
            half(layout.inputs) << 32 | half(layout.outputs),
            Ordering::Release,
        );
    }

    /// Reads the plugin's latency and tail, returning the latency.
    fn read(&self, plugin: &impl Plugin) -> u32 {
        let latency = plugin.latency();
        let tail = tail_answer(plugin.tail());
        self.latency.store(latency, Ordering::Release);
        self.tail.store(tail, Ordering::Release);
        latency
    }
}

/// What `tail` answers for `tail`.
fn tail_answer(tail: Tail) -> u32 {
    match tail {
        Tail::None => TAIL_NONE,
        // One frame short of forever, so a finite tail never reads as
        // infinite.
        Tail::Frames(frames) => frames.min(TAIL_INFINITE - 1),
        Tail::Infinite => TAIL_INFINITE,
    }
}

/// What only the main thread touches.
///
/// A host may call back in while the plugin is inside one of its calls to
/// the host, so each part is a `Cell` or a `RefCell`: a call that finds the
/// plugin or the drain borrowed refuses or skips, where a second `&mut`
/// would be undefined behaviour.
pub(crate) struct Main<P: Plugin, H> {
    plugin: RefCell<P>,
    drain: RefCell<HostEventDrain>,
    /// Read afresh for each call to the host: a release inside a tick
    /// clears it, so the tick calls the host no more.
    host: RefCell<Option<H>>,
    /// The next activation's setup, for a shell whose host sets it apart
    /// from activating (VST3's `setupProcessing`).
    #[cfg(feature = "vst3")]
    setup: Cell<Option<Setup>>,
    active: Cell<bool>,
    /// The latency the host last read or was told.
    reported: Cell<u32>,
    /// Inside a tick: a release from one of the host's callbacks then
    /// leaks the engine, since the tick is still using it.
    ticking: Cell<bool>,
    timer: RefCell<Option<Timer>>,
}

#[cfg(feature = "vst3")]
impl<P: Plugin, H> Main<P, H> {
    /// Whether a processor is built.
    pub(crate) fn active(&self) -> bool {
        self.active.get()
    }

    /// The setup the next activation is built for, as the shell last set it.
    pub(crate) fn setup(&self) -> Option<Setup> {
        self.setup.get()
    }
}

/// One instance of a plugin, as every exporter drives it.
///
/// The rules every shell shares:
///
/// - **Threads**, the threading rule in full ([`Plugin`] states it for plugin
///   authors, and `seam.h` for C users, at `SeamPlugin`). A shell builds the
///   engine bound to the calling thread ([`new`](Self::new)): VST3 at
///   `initialize`, the C interface at `create` for a plugin that is not `Send`.
///   A main-thread call on any other thread is refused. For a `Send` plugin,
///   the C interface builds the engine bound to no thread, for good
///   ([`new_unbound`](Self::new_unbound)). Each main-thread call then takes the
///   plugin on loan for its length, on whichever thread it arrives. One that
///   meets another thread's loan waits up to [`LOAN_WAIT`]
///   ([`main`](Self::main)) and is refused past it. A tick
///   ([`idle`](Self::idle)) skips instead, counting nothing, and a release
///   never waits (**Teardown**). A call nested inside another on the same
///   thread shares its loan. A refused call counts as a fault. The parameter
///   store, the saved block and the answers are safe from any thread. The audio
///   thread owns the processor and never waits.
/// - **Activation** builds a fresh processor for the setup and layout it is
///   given and hands it to the audio thread, replacing any running one,
///   which goes back to the plugin. An activation while active rebuilds, so
///   a processor that panicked recovers at any activation. A reset asked of
///   the processor it replaces is dropped: the new one has nothing to clear.
/// - **Host writes** from the audio thread always land. A write through a
///   controller lands too, except while the host says it is processing
///   (VST3's `setProcessing`). The host then sends the same value through
///   `process`, in order with its automation. Writing it here as well could
///   land it after a newer one. A shell with no processing state (the C
///   interface) never ignores one.
/// - **Reset** may be asked for on any thread; the next block runs it first.
/// - **Blocks.** A block that does not play (no processor, a processor that
///   panicked, or one an activation holds) is silence, which the shell
///   writes. One that plays answers its last piece's status.
/// - **Latency** is told at a tick when it moved since the host last read
///   it. Whether an activation counts as read is the shell's
///   [`HostLink::ACTIVATION_TELLS_LATENCY`].
/// - **Faults** are counted and reported as the [`faults`](crate::faults)
///   module says. An instance's are reported in [`idle`](Self::idle) and
///   [`release`](Self::release).
/// - **Teardown** ([`release`](Self::release)) never waits for the
///   main-thread state. On the main thread outside a tick, the processor
///   goes back to the plugin, open gestures are ended for the host, and the
///   engine is freed. From a host callback inside a tick, it deactivates,
///   calls the host no more, and leaks, since the tick goes on using it. A
///   release cannot take the state at once on a thread a bound engine
///   refuses, or while another thread's call has an unbound one's. Then it
///   counts a fault, the plugin and everything a tick can reach leak, and
///   the processor drops there, inside the panic guard.
pub(crate) struct Engine<P: Plugin, H> {
    /// Names the instance in its reports.
    number: u64,
    faults: Faults,
    set: Arc<ParamSet>,
    saved: Arc<SavedState>,
    /// Host-shaped writes, from the audio thread and from controllers.
    host: AudioParams,
    answers: Answers,
    /// Between the host's start and stop of processing, or the deactivation
    /// that ends it too.
    processing: AtomicBool,
    /// A reset asked for, which the next block runs first.
    reset: PendingReset,
    /// Set once the shell released the engine and it leaks: its tick does
    /// nothing more.
    released: AtomicBool,
    /// The timer's handle, for a release that leaks the timer to stop it
    /// from any thread.
    timer_handle: OnceLock<Handle>,
    /// Room for a block's automation, taken by the audio thread only.
    automation: AudioSlot<Automation>,
    audio: AudioSlot<Option<Audio<P::Processor>>>,
    main: MainThread<Main<P, H>>,
}

impl<P: Plugin, H: HostLink> Engine<P, H> {
    /// Creates the plugin on the calling thread, which becomes its main
    /// thread, linked to `host`.
    ///
    /// # Panics
    ///
    /// When the plugin's parameter set has its host event drain taken: the
    /// exporter owns it.
    pub(crate) fn new(host: Option<H>) -> Box<Self> {
        Self::build(host, MainThread::bound)
    }

    /// Creates the plugin on the calling thread, linked to `host`, and binds it
    /// to no thread, for good.
    ///
    /// [`main`](Self::main) then lends it to one main-thread call at a time, on
    /// whichever thread it arrives. A call that meets another thread's loan
    /// waits as `patience` says: a shell passes [`LOAN_WAIT`].
    ///
    /// # Panics
    ///
    /// As [`new`](Self::new).
    #[cfg(feature = "cabi")]
    pub(crate) fn new_unbound(host: Option<H>, patience: &'static dyn Patience) -> Box<Self>
    where
        P: Send,
        H: Send,
    {
        // SAFETY: of the main-thread state, the plugin and the host link
        // are `Send` (the bounds above). So are the event drain and the
        // `Cell`s and `RefCell`s around them. The one part that is not, the
        // timer slot, stays `None`: `start_timer` starts none for state no
        // thread owns. The state goes from thread to thread one loan at a
        // time, and drops wherever the engine does.
        Self::build(host, |main| unsafe { MainThread::unbound(main, patience) })
    }

    fn build(
        host: Option<H>,
        bind: impl FnOnce(Main<P, H>) -> MainThread<Main<P, H>>,
    ) -> Box<Self> {
        let plugin = P::new();
        let saved = plugin.saved();
        let set = Arc::clone(saved.set());
        let drain = set
            .take_host_event_drain()
            .expect("the exporter takes the set's host event drain; a plugin never does");
        let layout = const { first_layout(P::INFO.layouts) };
        let answers = Answers::new(layout, &plugin);
        let main = Main {
            reported: Cell::new(answers.latency.load(Ordering::Acquire)),
            plugin: RefCell::new(plugin),
            drain: RefCell::new(drain),
            host: RefCell::new(host),
            #[cfg(feature = "vst3")]
            setup: Cell::new(None),
            active: Cell::new(false),
            ticking: Cell::new(false),
            timer: RefCell::new(None),
        };
        let automation = Automation::new(set.len(), H::QUEUES);
        let engine = Box::new(Engine {
            number: INSTANCES.fetch_add(1, Ordering::Relaxed) + 1,
            faults: Faults::default(),
            host: set.audio(),
            set,
            saved,
            answers,
            processing: AtomicBool::new(false),
            reset: PendingReset::new(),
            released: AtomicBool::new(false),
            timer_handle: OnceLock::new(),
            automation: AudioSlot::new(automation),
            audio: AudioSlot::new(None),
            main: bind(main),
        });
        assert_send_sync(&*engine);
        engine
    }

    /// The plugin's parameter set. Any thread.
    pub(crate) fn set(&self) -> &Arc<ParamSet> {
        &self.set
    }

    /// The saved block the host's save and load go through. Any thread but
    /// audio.
    pub(crate) fn saved(&self) -> &SavedState {
        &self.saved
    }

    /// The main bus layout. Any thread.
    #[cfg(feature = "vst3")]
    pub(crate) fn layout(&self) -> Layout {
        self.answers.layout()
    }

    /// The latency the main thread last read. Any thread.
    pub(crate) fn latency(&self) -> u32 {
        self.answers.latency.load(Ordering::Acquire)
    }

    /// The tail the main thread last read, in frames, [`TAIL_NONE`] or
    /// [`TAIL_INFINITE`]. Any thread.
    pub(crate) fn tail(&self) -> u32 {
        self.answers.tail.load(Ordering::Acquire)
    }

    /// A host write that always lands: one the C interface's host makes, on
    /// any thread.
    #[cfg(feature = "cabi")]
    pub(crate) fn host_write(&self, id: ParamId, normalized: f64) {
        self.host.host_write(id, normalized);
    }

    /// A host write through a controller: ignored while the host is
    /// processing, since it sends the same value through `process`. Any
    /// thread.
    #[cfg(feature = "vst3")]
    pub(crate) fn controller_write(&self, id: ParamId, normalized: f64) {
        if !self.processing.load(Ordering::Acquire) {
            self.host.host_write(id, normalized);
        }
    }

    /// The host starts or stops processing. A stop asks the next block to
    /// reset the processor. Any thread: only atomics.
    #[cfg(feature = "vst3")]
    pub(crate) fn set_processing(&self, processing: bool) {
        self.processing.store(processing, Ordering::Release);
        if !processing {
            self.request_reset();
        }
    }

    /// Asks the next block to reset the processor first. Any thread.
    pub(crate) fn request_reset(&self) {
        self.reset.ask();
    }

    /// The main-thread state for one call, as the threading rule above
    /// says: for an engine made unbound, a loan to the calling thread until
    /// it drops. `None`, counted as a fault, when the rule refuses the call.
    pub(crate) fn main(&self) -> Option<Lent<'_, Main<P, H>>> {
        self.main
            .lend_or_wait()
            .map_err(|why| self.refused(why))
            .ok()
    }

    /// Counts a main-thread call refused for `why` as its fault.
    fn refused(&self, why: Refused) {
        self.faults.record(match why {
            Refused::Elsewhere => Fault::WrongThread,
            Refused::Lent => Fault::HeldElsewhere,
        });
    }

    /// Runs an entry point's body for this instance, answering `refused` if
    /// it panics, which counts as a fault.
    pub(crate) fn guard<R>(&self, refused: R, body: impl FnOnce() -> R) -> R {
        self.faults.guard(refused, body)
    }

    /// How many faults the instance has had, on every thread.
    #[cfg(any(test, feature = "test-host"))]
    pub(crate) fn faults(&self) -> u64 {
        self.faults.count()
    }

    /// The timer's handle, once it started.
    #[cfg(all(test, feature = "vst3"))]
    pub(crate) fn timer_handle(&self) -> Option<timer::Handle> {
        self.timer_handle.get().copied()
    }

    /// Every automation point the last block carried, and how many it had
    /// no room for.
    #[cfg(all(test, feature = "vst3"))]
    pub(crate) fn carried(&self) -> (Vec<Point>, u64) {
        let automation = self.automation.hold();
        (automation.points().to_vec(), automation.dropped())
    }

    /// Reports the faults not reported yet, as [`faults`](crate::faults)
    /// says. The caller is not the audio thread.
    fn report_faults(&self) {
        // Naming the instance allocates: only when there is something to say.
        if self.faults.pending() {
            self.faults
                .report(&format!("{} (instance {})", P::INFO.name, self.number));
        }
    }

    /// Sets the setup the next activation is built for; false while active,
    /// when it would not apply.
    #[cfg(feature = "vst3")]
    pub(crate) fn set_setup(&self, main: &Main<P, H>, setup: Setup) -> bool {
        if main.active.get() {
            return false;
        }
        main.setup.set(Some(setup));
        true
    }

    /// Sets the main bus layout for the next activation; false while active
    /// or for a layout the plugin does not declare.
    #[cfg(feature = "vst3")]
    pub(crate) fn arrange(&self, main: &Main<P, H>, layout: Layout) -> bool {
        if main.active.get() || !P::INFO.layouts.contains(&layout) {
            return false;
        }
        self.answers.set_layout(layout);
        true
    }

    /// Activates for `setup` and `layout`, as the activation rule above says.
    ///
    /// False for a layout the plugin does not declare, or when the plugin is
    /// busy in a call that reached the host, which counts as a fault.
    pub(crate) fn activate(&self, main: &Main<P, H>, setup: Setup, layout: Layout) -> bool {
        if !P::INFO.layouts.contains(&layout) {
            return false;
        }
        let Ok(mut plugin) = main.plugin.try_borrow_mut() else {
            self.faults.record(Fault::Reentered);
            return false;
        };
        let audio = Audio::new(&mut *plugin, setup, layout, |midi| {
            if midi { MAX_EVENTS } else { 0 }
        });
        let previous = install(&self.audio, &self.reset, audio);
        self.answers.set_layout(layout);
        main.active.set(true);
        if let Some(previous) = previous {
            plugin.deactivate(previous.processor);
        }
        let latency = self.answers.read(&*plugin);
        if H::ACTIVATION_TELLS_LATENCY {
            main.reported.set(latency);
        }
        true
    }

    /// Takes the processor back from the audio thread, once a running block
    /// finishes, and hands it to the plugin.
    ///
    /// Deactivating ends processing, though a host may not have said so. False
    /// when the plugin is busy in a call that reached the host, which counts as
    /// a fault.
    pub(crate) fn deactivate(&self, main: &Main<P, H>) -> bool {
        if !main.active.get() {
            return true;
        }
        let Ok(mut plugin) = main.plugin.try_borrow_mut() else {
            self.faults.record(Fault::Reentered);
            return false;
        };
        self.processing.store(false, Ordering::Release);
        let audio = self.audio.hold().take();
        main.active.set(false);
        if let Some(audio) = audio {
            plugin.deactivate(audio.processor);
        }
        self.answers.read(&*plugin);
        true
    }

    /// Links the host, first ending every gesture still open with the one
    /// it replaces.
    #[cfg(feature = "vst3")]
    pub(crate) fn connect(&self, main: &Main<P, H>, host: Option<H>) {
        self.finish_edits(main);
        *main.host.borrow_mut() = host;
    }

    /// Starts a timer that ticks the plugin about 60 times a second on this
    /// thread, until the engine is released.
    ///
    /// Only an engine bound to this thread starts one, since the timer may
    /// not leave it. One made unbound, or a system that refuses a timer,
    /// counts as a fault, and the timer then never ticks.
    #[cfg(feature = "vst3")]
    pub(crate) fn start_timer(&self) {
        let Some(main) = self.main.owned() else {
            self.faults.record(Fault::NoTimer);
            return;
        };
        match Timer::start(self.ticker()) {
            Some(timer) => {
                let _ = self.timer_handle.set(timer.handle());
                *main.timer.borrow_mut() = Some(timer);
            }
            None => self.faults.record(Fault::NoTimer),
        }
    }

    /// The tick as a timer holds it, reaching the engine through a pointer:
    /// a host may release the shell inside it.
    #[cfg(feature = "vst3")]
    pub(crate) fn ticker(&self) -> Box<dyn Fn()> {
        let engine = NonNull::from(self);
        Box::new(move || {
            // SAFETY: the engine is boxed and never moves. Its shell frees
            // it only after `release` dropped the timer that holds this
            // tick, on the main thread, outside any tick; otherwise it
            // leaks. Either way, the engine is live whenever a tick runs.
            unsafe { engine.as_ref() }.idle();
        })
    }

    /// One tick, as a shell or its timer calls it, under the threading rule
    /// above. A panic inside it is caught and counted.
    pub(crate) fn idle(&self) {
        self.guard((), || match self.main.lend() {
            Ok(main) => {
                self.report_faults();
                self.tick(&main);
            }
            Err(Refused::Lent) => {}
            Err(Refused::Elsewhere) => self.refused(Refused::Elsewhere),
        });
    }

    /// The plugin's idle call, then what it did, told to the host.
    ///
    /// A tick that a host's nested message loop delivers from inside one of the
    /// host's callbacks does nothing. The outer tick is still running, and the
    /// next one runs as usual.
    pub(crate) fn tick(&self, main: &Main<P, H>) {
        if main.ticking.get() || self.released.load(Ordering::Acquire) {
            return;
        }
        main.ticking.set(true);
        let _ticking = Clear(&main.ticking);
        match main.plugin.try_borrow_mut() {
            Ok(mut plugin) => plugin.idle(),
            // A host's callback into the plugin from inside one of its own:
            // the next tick runs instead.
            Err(_) => return,
        }
        self.tell_host(main);
    }

    /// Tells the host of the edits the plugin made since the last call, and
    /// of a sync's request to re-read every value or a latency that moved.
    fn tell_host(&self, main: &Main<P, H>) {
        if let Ok(mut drain) = main.drain.try_borrow_mut() {
            drain.drain(|event| self.emit(main, event));
        }
        let host = main.host.borrow().clone();
        let mut restart = Restart {
            // Taken only for a host to tell: with none linked yet, the
            // request waits for the tick after one is.
            values: host.is_some() && self.set.take_rescan(),
            latency: false,
        };
        if let Ok(plugin) = main.plugin.try_borrow() {
            let latency = self.answers.read(&*plugin);
            if latency != main.reported.get() {
                main.reported.set(latency);
                restart.latency = true;
            }
        }
        if let Some(host) = host
            && restart != Restart::default()
        {
            host.restart(restart);
        }
    }

    /// Ends every gesture the host still has open, as it stops listening.
    pub(crate) fn finish_edits(&self, main: &Main<P, H>) {
        if let Ok(mut drain) = main.drain.try_borrow_mut() {
            drain.finish(|event| self.emit(main, event));
        }
    }

    /// One host event, to the host linked now.
    fn emit(&self, main: &Main<P, H>, event: HostEvent) {
        let host = main.host.borrow().clone();
        if let Some(host) = host {
            host.tell(&self.set, event);
        }
    }

    /// Ends the instance as its shell is released or destroyed, and says
    /// whether the shell may free the engine (the teardown rule above).
    ///
    /// Why a release that cannot take the main-thread state leaks rather than
    /// wait: on a thread a bound engine refuses, the plugin may not drop there
    /// and the main thread may be ticking. Another thread's call on an unbound
    /// engine goes on using it until it returns, even after it lets the state
    /// go, so a release that waited would free the engine under it. Either
    /// way a tick may call the host once more after the release, which only a
    /// host that breaks the threading rules meets; the engine leaks, so
    /// Seam's side stays memory-safe. A VST3 handler is held by the engine's
    /// own reference. The C interface's callbacks, though, are the host's,
    /// which `seam.h` keeps valid only until `destroy` returns.
    pub(crate) fn release(&self) -> Release {
        self.guard(Release::Leak, || {
            let main = match self.main.lend() {
                Ok(main) => main,
                Err(why) => {
                    self.refused(why);
                    return self.leak_refused();
                }
            };
            if main.ticking.get() {
                self.leave();
                main.host.borrow_mut().take();
                self.deactivate(&main);
                return Release::Leak;
            }
            main.timer.borrow_mut().take();
            self.deactivate(&main);
            self.finish_edits(&main);
            self.report_faults();
            Release::Free
        })
    }

    /// A release refused the main-thread state: the plugin and everything a
    /// tick can reach leak.
    fn leak_refused(&self) -> Release {
        // Not the audio thread, which a host never releases an instance
        // from, so the report may write here.
        self.report_faults();
        self.leave();
        // The plugin may not drop here, but the processor may: inside the
        // release's guard, so a panic in its drop is caught.
        drop(self.audio.hold().take());
        Release::Leak
    }

    /// Marks the engine as leaking, so its tick does nothing more, and
    /// detaches the timer, which leaks with it, from its tick.
    fn leave(&self) {
        self.released.store(true, Ordering::Release);
        if let Some(&handle) = self.timer_handle.get() {
            abandon(handle);
        }
    }
}

/// The layout an instance starts with, the first of `layouts`. Evaluated in
/// a `const` block, it fails to compile for a plugin that declares none,
/// since every activation would then be refused.
pub(crate) const fn first_layout(layouts: &[Layout]) -> Layout {
    match layouts.first() {
        Some(&layout) => layout,
        None => panic!("a plugin declares at least one layout"),
    }
}

/// Fails to compile unless the engine is `Send` and `Sync`, as its shell
/// shares it with every thread a host calls from.
fn assert_send_sync<T: Send + Sync>(_: &T) {}

/// Clears a flag when dropped, even by a panic.
struct Clear<'a>(&'a Cell<bool>);

impl Drop for Clear<'_> {
    fn drop(&mut self) {
        self.0.set(false);
    }
}

impl<P: Plugin, H> std::fmt::Debug for Engine<P, H> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Engine")
            .field("plugin", &P::INFO.name)
            .finish_non_exhaustive()
    }
}

// What the engine answers before any exporter is involved, so these build
// with no feature, unlike `tests`, which drives the engine through them.
#[cfg(test)]
mod core_tests {
    use super::{MAX_FRAMES, Setup, TAIL_INFINITE, TAIL_NONE, Tail, tail_answer};

    #[test]
    fn a_rate_that_is_not_positive_and_finite_builds_no_setup() {
        for rate in [0.0, -0.0, -1.0, f64::INFINITY, f64::NEG_INFINITY, f64::NAN] {
            assert_eq!(Setup::new(rate, 512), None, "{rate}");
        }
        assert!(Setup::new(f64::MIN_POSITIVE, 512).is_some());
    }

    #[test]
    fn a_block_size_of_0_takes_the_fallbacks_and_a_longer_one_the_most() {
        let fallback = Setup::new(48_000.0, 0).expect("a good rate");
        assert_eq!(fallback.sample_rate, 48_000.0, "the rate is kept");
        assert_eq!(fallback.max_frames, Setup::FALLBACK.max_frames);
        for frames in [MAX_FRAMES + 1, usize::MAX] {
            let setup = Setup::new(96_000.0, frames).expect("a good rate");
            assert_eq!(setup.sample_rate, 96_000.0, "the rate is kept");
            assert_eq!(setup.max_frames.get(), MAX_FRAMES, "{frames}");
        }
    }

    #[test]
    fn a_finite_tail_never_answers_as_infinite() {
        assert_eq!(tail_answer(Tail::None), TAIL_NONE);
        assert_eq!(tail_answer(Tail::Frames(48_000)), 48_000);
        assert_eq!(tail_answer(Tail::Frames(u32::MAX)), u32::MAX - 1);
        assert_eq!(tail_answer(Tail::Infinite), TAIL_INFINITE);
    }
}
