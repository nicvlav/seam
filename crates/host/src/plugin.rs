//! The traits a plugin implements once for every format: [`Plugin`] on the
//! main thread, and the [`Processor`] it builds for the audio thread.

use std::sync::Arc;

use seam_params::ParamDecl;

use crate::audio::{Block, ProcessSetup, Status};
use crate::info::PluginInfo;
use crate::state::SavedState;

/// A plugin, written once for every format.
///
/// A plugin lives on its main thread: the exporter creates it, calls it and
/// drops it there, one call at a time. It may therefore own state that is
/// not `Send`: its undo history, the session's UI handle, learn, an `Rc`.
/// Which thread runs what:
///
/// - **Main thread:** every method here, and the plugin's drop. It is the
///   thread that initializes a VST3 instance or calls the C interface's
///   `create`. A call that arrives on another thread is refused.
/// - **Any thread, one call at a time:** a plugin that is `Send` has no main
///   thread through the C interface. Where a method says "Thread: main",
///   each call, and the drop, may come on a different thread. Two never run
///   at once, and each sees what the one before did. Such a plugin holds
///   nothing tied to a thread, such as a thread-local or a window handle.
///   `cabi::SeamPlugin` states the C contract. Through VST3 it has a main
///   thread, as above.
/// - **Audio thread:** only the [`Processor`]'s `process` and `reset`. The
///   processor is built by [`activate`](Self::activate) and handed back to
///   [`deactivate`](Self::deactivate), so it never drops on the audio
///   thread.
/// - **Whatever thread they arrive on:** the host's save and load calls,
///   which go through [`saved`](Self::saved) and never touch the plugin, and
///   the parameter set's reads and writes.
///
/// A host that releases an instance on another thread, against these rules,
/// leaks it, and the plugin is never dropped. A release from one of the
/// host's callbacks inside [`idle`](Self::idle) is allowed, but leaks the
/// instance too, deactivated, since the tick is still running.
///
/// What the exporters do for every plugin, so none writes it:
///
/// - **Bypass.** A parameter declared with
///   [`as_bypass`](seam_params::ParamDecl::as_bypass) is the one the host
///   drives as bypass, and the processor implements it. A plugin that
///   declares none leaves bypass to the host. None of Seam's exporters adds a
///   hidden parameter, though JUCE's VST3 wrapper does when the JUCE adapter
///   hosts the plugin.
/// - **Programs.** A plugin has one program, and the exporters offer the
///   host none to choose.
/// - **Samples.** 32-bit float only: an exporter refuses a host that asks
///   for 64-bit processing.
/// - **Denormals.** An exporter flushes denormals to zero around every
///   [`Processor::process`] call and restores the host's setting after it.
/// - **Host events.** The exporter takes the parameter set's
///   [`take_host_event_drain`](seam_params::ParamSet::take_host_event_drain)
///   drain and tells the host of every edit; a plugin never takes it.
/// - **Panics.** A panic never unwinds into the host. An exporter catches
///   it at the entry point and answers the host with an error, and a
///   processor that panicked plays silence until the next activation. This
///   needs `panic = "unwind"`, the default.
///
/// # Examples
///
/// A plugin with one parameter that passes audio through:
///
/// ```
/// use std::sync::Arc;
///
/// use seam_host::{
///     AuCodes, Block, Category, EventList, Layout, Plugin, PluginInfo, ProcessSetup, Processor,
///     SavedState, Status,
/// };
/// use seam_params::{Kind, ParamDecl, ParamSet};
/// use seam_session::Session;
///
/// struct Passthrough;
///
/// impl Processor for Passthrough {
///     fn process(&mut self, _block: Block<'_, '_>) -> Status {
///         Status::Continue
///     }
/// }
///
/// struct Gain {
///     saved: Arc<SavedState>,
/// }
///
/// impl Plugin for Gain {
///     const INFO: PluginInfo = PluginInfo {
///         name: "Gain",
///         vendor: "Example",
///         url: "",
///         email: "",
///         description: "",
///         version: "0.1.0",
///         id: "dev.example.gain",
///         vst3_class_id: [0; 16],
///         au_codes: AuCodes { subtype: *b"gain", manufacturer: *b"Exmp" },
///         category: Category::Effect,
///         features: &[],
///         layouts: &[Layout::STEREO],
///         midi_input: false,
///         midi_output: false,
///     };
///     const PARAMS: &'static [ParamDecl] = &[
///         ParamDecl::new("gain", "Gain", Kind::continuous(-24.0, 24.0, 0.0, None), 0.0)
///             .with_unit("dB"),
///     ];
///     type Processor = Passthrough;
///
///     fn new() -> Self {
///         let set = ParamSet::from_decls(Self::PARAMS.iter().copied())
///             .expect("valid declarations");
///         let session = Session::new(&set);
///         Gain { saved: Arc::new(SavedState::new(&set, &session)) }
///     }
///
///     fn saved(&self) -> Arc<SavedState> {
///         Arc::clone(&self.saved)
///     }
///
///     fn activate(&mut self, _setup: &ProcessSetup) -> Passthrough {
///         Passthrough
///     }
/// }
///
/// let mut plugin = Gain::new();
/// let setup = ProcessSetup {
///     sample_rate: 48_000.0,
///     max_frames: 64,
///     inputs: 2,
///     outputs: 2,
///     max_events: 0,
///     max_out_events: 0,
/// };
/// let mut processor = plugin.activate(&setup);
///
/// let (mut left, mut right) = ([0.5_f32; 64], [0.5_f32; 64]);
/// let mut channels = [&mut left[..], &mut right[..]];
/// let (events, mut out) = (EventList::with_capacity(0), EventList::with_capacity(0));
/// let block = Block::new(&mut channels, 64, &events, &mut out);
/// assert_eq!(processor.process(block), Status::Continue);
/// ```
pub trait Plugin: 'static {
    /// The facts a host reads before it creates an instance.
    const INFO: PluginInfo;

    /// Every parameter, in declaration order, which is the order hosts list
    /// them in. Hosts identify a parameter by a hash of its key, so the
    /// order may change between versions.
    const PARAMS: &'static [ParamDecl];

    /// The audio-thread half.
    type Processor: Processor;

    /// Creates an instance.
    ///
    /// Thread: main, or for a plugin that is `Send` the thread that calls
    /// the C interface's `create`.
    fn new() -> Self;

    /// Returns the saved state that the host's save and load calls go
    /// through.
    ///
    /// The exporter calls it once, after [`new`](Self::new), and keeps the
    /// handle for the instance's life. The exporter takes the plugin's
    /// parameter set from that handle ([`SavedState::set`]), so it never
    /// builds a second one from [`PARAMS`](Self::PARAMS).
    ///
    /// Thread: main.
    fn saved(&self) -> Arc<SavedState>;

    /// Builds the processor for `setup`.
    ///
    /// Everything the audio thread will touch is allocated here, at a fixed
    /// capacity.
    ///
    /// Thread: main.
    fn activate(&mut self, setup: &ProcessSetup) -> Self::Processor;

    /// Takes the processor back when the host deactivates.
    ///
    /// What it holds is freed here and never on the audio thread. By default
    /// it is dropped.
    ///
    /// Thread: main.
    fn deactivate(&mut self, processor: Self::Processor) {
        drop(processor);
    }

    /// Returns the processing delay, in frames, for the host to compensate.
    ///
    /// The exporter reads it at creation, after each activation and each
    /// deactivation, and at each [`idle`](Self::idle) tick, and tells the
    /// host when it changes.
    ///
    /// Thread: main.
    fn latency(&self) -> u32 {
        0
    }

    /// Returns how long the output can still sound after the input goes
    /// silent, for a host that stops processing a quiet plugin.
    ///
    /// Thread: main.
    fn tail(&self) -> Tail {
        Tail::None
    }

    /// Handles the main-thread tick, about 60 times a second from creation to
    /// destruction, editor open or not.
    ///
    /// Afterwards the exporter tells the host of the edits the plugin made;
    /// nothing else of the plugin's runs on a tick unless it calls it here.
    ///
    /// A plugin built on Seam's crates calls, in this order:
    ///
    /// 1. `Learn::poll`, when it uses MIDI learn: it commits a mapping
    ///    learned since the last tick and writes the learned values.
    /// 2. [`SessionUi::tick`](seam_session::SessionUi::tick): it captures
    ///    what changed for the next save, and applies a session a host's
    ///    load restored. Learn's poll holds learn as it commits a mapping, so
    ///    the session can capture it only at its next tick. Polled after the
    ///    tick, a new mapping misses the save until the tick after.
    /// 3. `History::tick`, when it keeps undo: it records what changed since
    ///    the last tick. Its place does not matter: it clears the history
    ///    whenever a restore was stored or applied since it last looked.
    ///
    /// Surfaces that show values poll their [watches](seam_params::Watch) on
    /// the tick too, in any order with these.
    ///
    /// Thread: main. The JUCE adapter ticks it on every platform it builds
    /// for, the VST3 exporter with its own timer on Windows and macOS, the
    /// only platforms it builds for.
    ///
    /// Keep a tick short. Through the C interface, a main-thread call that
    /// meets a tick on another thread waits for it only briefly (its
    /// `SeamPlugin` says how long). An activation that meets a slow tick
    /// then fails.
    ///
    /// # Examples
    ///
    /// ```
    /// use std::cell::RefCell;
    /// use std::rc::Rc;
    /// use std::sync::Arc;
    ///
    /// use seam_host::{Block, Plugin, ProcessSetup, Processor, SavedState, Status};
    /// use seam_learn::{Config, Learn, LearnAudio, SESSION_KEY, Takes};
    /// use seam_params::{Kind, ParamDecl, ParamSet, UiParams};
    /// use seam_session::{Session, SessionUi};
    /// use seam_undo::{Bound, History};
    /// # use seam_host::{AuCodes, Category, Layout, PluginInfo};
    /// # use seam_ump::{ChannelVoice, EventList, Voice};
    ///
    /// /// The processor: learn's audio side reads each block's MIDI first.
    /// struct Voices {
    ///     learn: LearnAudio,
    /// }
    ///
    /// impl Processor for Voices {
    ///     fn process(&mut self, block: Block<'_, '_>) -> Status {
    ///         self.learn.process(block.events);
    ///         Status::Sleep
    ///     }
    /// }
    ///
    /// struct Synth {
    ///     saved: Arc<SavedState>,
    ///     /// What the plugin's page and controls write through.
    ///     params: UiParams,
    ///     session: SessionUi,
    ///     learn: Rc<RefCell<Learn>>,
    ///     history: History,
    /// }
    ///
    /// impl Plugin for Synth {
    /// #   const INFO: PluginInfo = PluginInfo {
    /// #       name: "Synth",
    /// #       vendor: "Example",
    /// #       url: "",
    /// #       email: "",
    /// #       description: "",
    /// #       version: "0.1.0",
    /// #       id: "dev.example.synth",
    /// #       vst3_class_id: [0; 16],
    /// #       au_codes: AuCodes { subtype: *b"synt", manufacturer: *b"Exmp" },
    /// #       category: Category::Instrument,
    /// #       features: &[],
    /// #       layouts: &[Layout::new(0, 2)],
    /// #       midi_input: true,
    /// #       midi_output: false,
    /// #   };
    /// #   const PARAMS: &'static [ParamDecl] = &[ParamDecl::new(
    /// #       "cutoff",
    /// #       "Cutoff",
    /// #       Kind::continuous(20.0, 20_000.0, 0.0, None),
    /// #       1_000.0,
    /// #   )];
    ///     type Processor = Voices;
    ///
    ///     fn new() -> Self {
    ///         let set = ParamSet::from_decls(Self::PARAMS.iter().copied()).expect("valid");
    ///         let session = Session::new(&set);
    ///         let ui = session.take_ui().expect("the first session UI handle");
    ///         // Each activation takes an audio side of its own, in `activate`.
    ///         let (learn, _) = Learn::new(&set, Config::default());
    ///         let learn = Rc::new(RefCell::new(learn));
    ///         Learn::attach(&learn, &ui).expect("learn's key is free");
    ///         let params = set.take_ui().expect("the first UI handle");
    ///         let history = History::new(&params, Bound::default()).expect("the first history");
    ///         history.enroll_params();
    ///         Synth {
    ///             saved: Arc::new(SavedState::new(&set, &session)),
    ///             params,
    ///             session: ui,
    ///             learn,
    ///             history,
    ///         }
    ///     }
    ///
    ///     fn activate(&mut self, _setup: &ProcessSetup) -> Voices {
    ///         Voices { learn: self.learn.borrow_mut().replace_audio() }
    ///     }
    ///
    ///     fn idle(&mut self) {
    ///         let _learned = self.learn.borrow_mut().poll();
    ///         self.session.tick();
    ///         self.history.tick();
    ///     }
    /// #
    /// #   fn saved(&self) -> Arc<SavedState> {
    /// #       Arc::clone(&self.saved)
    /// #   }
    /// }
    ///
    /// let mut synth = Synth::new();
    /// let cutoff = synth.saved().set().id("cutoff").ok_or("declared")?;
    ///
    /// // The user drags the cutoff on the page: one undo step at the next tick.
    /// synth.params.write(cutoff, 500.0);
    /// synth.idle();
    /// assert_eq!(synth.history.len(), 1);
    ///
    /// // The user arms learn and turns controller 74, which the host's block brings.
    /// # let setup = ProcessSetup {
    /// #     sample_rate: 48_000.0,
    /// #     max_frames: 1,
    /// #     inputs: 0,
    /// #     outputs: 2,
    /// #     max_events: 1,
    /// #     max_out_events: 0,
    /// # };
    /// let mut voices = synth.activate(&setup);
    /// synth.learn.borrow_mut().arm(cutoff, Takes::CONTROLLER)?;
    /// # let voice = Voice::Midi1ControlChange { controller: 74, value: 64 };
    /// # let mut events = EventList::with_capacity(1);
    /// # events.push(ChannelVoice { group: 0, channel: 0, voice }.event(0));
    /// # let (mut left, mut right) = ([0.0_f32; 1], [0.0_f32; 1]);
    /// # let mut out = EventList::with_capacity(0);
    /// # let mut channels = [&mut left[..], &mut right[..]];
    /// voices.process(Block::new(&mut channels, 1, &events, &mut out));
    /// synth.idle();
    ///
    /// // One tick, and the host's next save holds the mapping.
    /// let block: serde_json::Value = serde_json::from_slice(&synth.saved().save())?;
    /// assert_eq!(block["extra"][SESSION_KEY]["mappings"][0]["target"], "cutoff");
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    fn idle(&mut self) {}
}

/// How long a plugin's output can outlast its input.
///
/// # Examples
///
/// ```
/// use seam_host::Tail;
///
/// /// How long a plugin that reports `tail` keeps sounding, in seconds.
/// fn ring_seconds(tail: Tail, sample_rate: f64) -> Option<f64> {
///     match tail {
///         Tail::None => Some(0.0),
///         Tail::Frames(frames) => Some(f64::from(frames) / sample_rate),
///         Tail::Infinite => None,
///     }
/// }
///
/// assert_eq!(ring_seconds(Tail::Frames(96_000), 48_000.0), Some(2.0));
/// assert_eq!(ring_seconds(Tail::Infinite, 48_000.0), None);
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tail {
    /// Silent input gives silent output at once.
    None,
    /// This many frames at the current sample rate. `u32::MAX`, which the
    /// formats spell as infinite, is answered one frame less.
    Frames(u32),
    /// It may never fall silent: a host keeps processing.
    Infinite,
}

/// The audio-thread half of a plugin.
///
/// # Examples
///
/// ```
/// use seam_host::{Block, EventList, Processor, Status};
///
/// /// A one-pole low-pass: it carries one sample from block to block.
/// struct OnePole {
///     state: f32,
/// }
///
/// impl Processor for OnePole {
///     fn process(&mut self, block: Block<'_, '_>) -> Status {
///         for channel in block.channels.iter_mut() {
///             for sample in channel.iter_mut() {
///                 self.state += 0.5 * (*sample - self.state);
///                 *sample = self.state;
///             }
///         }
///         Status::Continue
///     }
///
///     fn reset(&mut self) {
///         self.state = 0.0;
///     }
/// }
///
/// let mut filter = OnePole { state: 0.0 };
/// let (mut left, mut right) = ([1.0_f32; 4], [1.0_f32; 4]);
/// let mut channels = [&mut left[..], &mut right[..]];
/// let (events, mut out) = (EventList::with_capacity(0), EventList::with_capacity(0));
/// filter.process(Block::new(&mut channels, 4, &events, &mut out));
/// assert_eq!(left, [0.5, 0.75, 0.875, 0.9375]);
/// filter.reset();
/// assert_eq!(filter.state, 0.0);
/// ```
pub trait Processor: Send + 'static {
    /// Processes one block: its audio in place, its MIDI in, and any MIDI it
    /// sends (see [`Block`]).
    ///
    /// Audio is in place because a host may hand the same buffers for input
    /// and output, and two Rust slices over the same memory are undefined
    /// behaviour. The exporter therefore copies the input in only when the
    /// host's buffers are separate. Even the first block after activation
    /// allocates nothing.
    ///
    /// Open: no exporter carries a block's MIDI either way yet, as
    /// [`PluginInfo::midi_input`] says.
    ///
    /// Thread: audio. Never allocates, locks or makes a system call.
    fn process(&mut self, block: Block<'_, '_>) -> Status;

    /// Clears what the processor carries between blocks (tails, smoothing),
    /// without reallocating.
    ///
    /// The exporter calls it before the next block, after the host stops
    /// processing.
    ///
    /// Thread: audio.
    fn reset(&mut self) {}
}
