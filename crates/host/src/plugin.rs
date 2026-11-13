//! The traits a plugin implements once for every format: [`Plugin`] on the
//! main thread, and the [`Processor`] it builds for the audio thread.

use std::error::Error;

use seam_params::Params;
use serde_json::{Map, Value as Json};

use crate::audio::{Block, ProcessSetup, Status};
use crate::editor::Editor;
use crate::info::PluginInfo;
use crate::parts::Parts;

/// A plugin, written once for every format.
///
/// A plugin lives on its main thread: the exporter creates it, calls it and
/// drops it there, one call at a time. It may therefore own state that is
/// not `Send`: its [`Parts`], its undo history, an `Rc`. Which thread runs
/// what:
///
/// - **Main thread:** every method here, and the plugin's drop. It is the
///   thread that initializes a VST3 instance or calls the C interface's
///   `create`. A call that arrives on another thread is refused.
/// - **Any thread, one call at a time:** a plugin that is `Send` has no main
///   thread through the C interface. Where a method says "Thread: main",
///   each call, and the drop, may come on a different thread. Two never run
///   at once, and each sees what the one before did. Such a plugin holds
///   nothing tied to a thread, such as a thread-local, a window handle or
///   its [`Parts`]. `cabi::SeamPlugin` states the C contract. Through VST3
///   it has a main thread, as above.
/// - **Audio thread:** only the [`Processor`]'s `process` and `reset`. The
///   processor is built by [`activate`](Self::activate) and handed back to
///   [`deactivate`](Self::deactivate), so it never drops on the audio
///   thread.
/// - **Whatever thread they arrive on:** the host's save and load calls,
///   which go through the saved state Seam built ([`Parts::saved`]) and
///   never touch the plugin, a load's [`migrate`](Self::migrate), and the
///   parameter set's reads and writes.
///
/// A host that releases an instance on another thread, against these rules,
/// leaks it, and the plugin is never dropped. A release from one of the
/// host's callbacks inside [`idle`](Self::idle) is allowed, but leaks the
/// instance too, deactivated, since the tick is still running.
///
/// What the exporters do for every plugin, so none writes it:
///
/// - **Parts.** Seam declares the plugin's [`Params`](Self::Params), builds
///   the parameter set, the session and the saved state over them, and hands
///   the parameters and the [`Parts`] to [`new`](Self::new). The host lists
///   that set, and saves and loads through that saved state. A declaration
///   refused fails the instance's creation, with the reason in the fault
///   report. While the plugin keeps the parts, Seam ticks them before each
///   [`idle`](Self::idle).
/// - **Learn.** When the plugin turned learn on
///   ([`Parts::midi_learn`]), each activation takes a fresh audio side of
///   learn, and each block runs it over the block's MIDI before the
///   processor's first call, so learned writes are in the parameters it
///   reads. The processor holds no part of learn.
/// - **Bypass.** A parameter declared with
///   [`as_bypass`](seam_params::BoolDecl::as_bypass) is the one the host
///   drives as bypass, and the processor implements it. A plugin that
///   declares none leaves bypass to the host. None of Seam's exporters adds a
///   hidden parameter, though JUCE's VST3 wrapper does when the JUCE adapter
///   hosts the plugin.
/// - **Editor.** When the plugin hands out an [`Editor`] from
///   [`editor`](Self::editor), the VST3 exporter opens it in the window the
///   host makes, drives its size and scale, ticks it after each
///   [`idle`](Self::idle) while it is open, and closes it with the window. A
///   plugin writes no pump and no timer for it. The C interface shows no
///   editor.
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
/// A plugin with one parameter, a gain its processor applies:
///
/// ```
/// use seam_host::{
///     AuCodes, Block, Category, EventList, Layout, Parts, Plugin, PluginInfo, ProcessSetup,
///     Processor, Status,
/// };
/// use seam_params::{Declarer, Float, FloatDecl, Params};
///
/// /// The parameters: declared here, and handed back to the plugin.
/// #[derive(Clone)]
/// struct GainParams {
///     gain: Float,
/// }
///
/// impl Params for GainParams {
///     fn declare(p: &mut Declarer) -> Self {
///         GainParams {
///             gain: p.add(FloatDecl::new("gain", "Gain", -24.0..=24.0, 0.0).with_unit("dB")),
///         }
///     }
/// }
///
/// /// The audio-thread half: it reads the gain through its own clone.
/// struct Level(GainParams);
///
/// impl Processor for Level {
///     fn process(&mut self, block: Block<'_, '_>) -> Status {
///         let gain = 10.0_f32.powf(self.0.gain.get() / 20.0);
///         for channel in block.channels.iter_mut() {
///             channel.iter_mut().for_each(|sample| *sample *= gain);
///         }
///         Status::Continue
///     }
/// }
///
/// /// The main-thread half. It keeps no parts, so it is `Send`.
/// struct Gain {
///     params: GainParams,
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
///     type Params = GainParams;
///     type Processor = Level;
///
///     fn new(params: GainParams, _parts: Parts) -> Self {
///         Gain { params }
///     }
///
///     fn activate(&mut self, _setup: &ProcessSetup) -> Level {
///         Level(self.params.clone())
///     }
/// }
///
/// // What an exporter does, here with no host.
/// let (params, parts) = Parts::for_plugin::<Gain>()?;
/// let mut plugin = Gain::new(params, parts);
/// let setup = ProcessSetup {
///     sample_rate: 48_000.0,
///     max_frames: 64,
///     layout: Layout::STEREO,
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
/// assert_eq!(left[0], 0.5, "0 dB");
/// # Ok::<(), seam_params::DeclError>(())
/// ```
pub trait Plugin: 'static {
    /// The facts a host reads before it creates an instance.
    const INFO: PluginInfo;

    /// The parameters a host is shown: a struct that declares each one in
    /// [`Params::declare`], in the order hosts list them within their group.
    /// `()` declares none.
    ///
    /// Hosts identify a parameter by a hash of its key, so the order may
    /// change between versions.
    type Params: Params;

    /// The version of the plugin's own saved-state schema, which a saved
    /// block carries apart from Seam's.
    ///
    /// It stays 0, the default, until a block of the old shape would load
    /// wrong: a parameter renamed or rescaled, a section restructured. The
    /// plugin then raises it, and upgrades a block of an older version in
    /// [`migrate`](Self::migrate).
    const SCHEMA_VERSION: u64 = 0;

    /// The audio-thread half.
    type Processor: Processor;

    /// Creates an instance from what Seam built for it: the parameters its
    /// [`Params`](Self::Params) declared, and its [`Parts`].
    ///
    /// The plugin keeps what it uses. Seam ticks the parts for as long as
    /// the plugin keeps them.
    ///
    /// Thread: main, or for a plugin that is `Send` the thread that calls
    /// the C interface's `create`.
    fn new(params: Self::Params, parts: Parts) -> Self;

    /// Upgrades a block an older schema saved, in place, from its version
    /// `from`.
    ///
    /// Seam calls it only while [`SCHEMA_VERSION`](Self::SCHEMA_VERSION) is
    /// above 0, for a block of a lower version, with the block as a JSON
    /// object holding its `parameters` and `extra`. The rules a migration
    /// keeps, such as never overwriting a key the block holds, are on
    /// [`SavedState::with_migration`](crate::SavedState::with_migration),
    /// whose example is one. By default it changes nothing.
    ///
    /// It runs inside the host's load, on whichever thread the host loads
    /// from, which is why it has no `self`: it touches only the block.
    ///
    /// # Errors
    ///
    /// Whatever the plugin returns, as the reason the host's load is
    /// refused. The load then changes nothing.
    fn migrate(
        from: u64,
        block: &mut Map<String, Json>,
    ) -> Result<(), Box<dyn Error + Send + Sync>> {
        let _ = (from, block);
        Ok(())
    }

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
    /// destruction, editor open or not, for the plugin's own work. By
    /// default it does nothing.
    ///
    /// Seam's own work comes first, in the same tick: when the plugin keeps
    /// its [`Parts`], Seam [ticks](crate::Driver::tick) them, which applies a
    /// session the host's load restored and runs undo and learn. So `idle`
    /// sees a restore applied and a learned mapping committed. Afterwards
    /// the exporter tells the host of the edits the plugin made. A tick that
    /// meets the plugin busy in a call of its own runs neither, and the next
    /// tick runs both.
    ///
    /// A surface the plugin draws itself polls its
    /// [watches](seam_params::Watch) here. An open [`Editor`]'s own tick
    /// comes after this one, in the same tick.
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
    /// A synth with undo and MIDI learn, whose idle shows whether its cutoff
    /// is learned. One tick records a drag; the next commits a learned
    /// controller and saves its mapping, before the synth's idle sees it.
    ///
    /// ```
    /// use seam_host::{Block, Driver, Parts, Plugin, ProcessSetup, Processor, Status};
    /// use seam_learn::{Config, SESSION_KEY, Takes};
    /// use seam_params::{Declarer, Float, FloatDecl, Params};
    /// use seam_undo::{Bound, History};
    /// # use seam_host::{AuCodes, Category, Layout, PluginInfo};
    /// # use seam_ump::{ChannelVoice, EventList, Voice};
    ///
    /// struct SynthParams {
    ///     cutoff: Float,
    /// }
    ///
    /// impl Params for SynthParams {
    ///     fn declare(p: &mut Declarer) -> Self {
    ///         SynthParams {
    ///             cutoff: p.add(FloatDecl::new("cutoff", "Cutoff", 20.0..=20_000.0, 1_000.0)),
    ///         }
    ///     }
    /// }
    ///
    /// /// The processor. Learned controllers have written the cutoff by the
    /// /// time it runs.
    /// struct Voices;
    ///
    /// impl Processor for Voices {
    ///     fn process(&mut self, _block: Block<'_, '_>) -> Status {
    ///         Status::Sleep
    ///     }
    /// }
    ///
    /// struct Synth {
    ///     parts: Parts,
    ///     params: SynthParams,
    ///     history: History,
    ///     /// What the editor shows beside the cutoff.
    ///     cutoff_learned: bool,
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
    ///     type Params = SynthParams;
    ///     type Processor = Voices;
    ///
    ///     fn new(params: SynthParams, mut parts: Parts) -> Self {
    ///         let history = parts.undo(Bound::default());
    ///         parts.midi_learn(Config::default());
    ///         Synth { parts, params, history, cutoff_learned: false }
    ///     }
    ///
    ///     fn activate(&mut self, _setup: &ProcessSetup) -> Voices {
    ///         Voices
    ///     }
    ///
    ///     fn idle(&mut self) {
    ///         let cutoff = self.params.cutoff.id();
    ///         self.cutoff_learned = self.parts.learn().mapping(cutoff).is_some();
    ///     }
    /// }
    ///
    /// // Built and ticked with no host, as a test does: Seam's tick, then idle.
    /// let (params, parts) = Parts::for_plugin::<Synth>()?;
    /// let driver = Driver::of(&parts);
    /// let mut synth = Synth::new(params, parts);
    /// let mut tick = |synth: &mut Synth| {
    ///     driver.tick();
    ///     synth.idle();
    /// };
    /// let cutoff = synth.params.cutoff.id();
    ///
    /// // The user drags the cutoff on the page: one undo step at the next tick.
    /// synth.parts.ui().write(cutoff, 500.0);
    /// tick(&mut synth);
    /// assert_eq!(synth.history.len(), 1);
    ///
    /// // The user arms learn and turns controller 74, which the host's block
    /// // brings. Learn's audio side runs over the block before the processor,
    /// // as the engine runs it.
    /// # let setup = ProcessSetup {
    /// #     sample_rate: 48_000.0,
    /// #     max_frames: 1,
    /// #     layout: Layout::new(0, 2),
    /// #     max_events: 1,
    /// #     max_out_events: 0,
    /// # };
    /// let mut voices = synth.activate(&setup);
    /// let mut learn = driver.learn_audio().expect("learn is on");
    /// synth.parts.learn().arm(cutoff, Takes::CONTROLLER)?;
    /// # let voice = Voice::Midi1ControlChange { controller: 74, value: 64 };
    /// # let mut events = EventList::with_capacity(1);
    /// # events.push(ChannelVoice { group: 0, channel: 0, voice }.event(0));
    /// # let (mut left, mut right) = ([0.0_f32; 1], [0.0_f32; 1]);
    /// # let mut out = EventList::with_capacity(0);
    /// # let mut channels = [&mut left[..], &mut right[..]];
    /// learn.process(&events);
    /// voices.process(Block::new(&mut channels, 1, &events, &mut out));
    /// assert!(!synth.cutoff_learned);
    /// tick(&mut synth);
    /// assert!(synth.cutoff_learned);
    ///
    /// // One tick, and the host's next save holds the mapping.
    /// let block: serde_json::Value = serde_json::from_slice(&synth.parts.saved().save())?;
    /// assert_eq!(block["extra"][SESSION_KEY]["mappings"][0]["target"], "cutoff");
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    fn idle(&mut self) {}

    /// Returns an editor for the window a host makes for the plugin, or
    /// `None`, the default, for a plugin the host shows with its own
    /// controls.
    ///
    /// Seam calls it each time a host asks for a view. The editor keeps what
    /// it needs of the plugin, a clone of [`Parts::ui`] for its writes among
    /// them, since the exporter drives it apart from the plugin: [`Editor`]
    /// says how.
    ///
    /// Thread: main.
    ///
    /// # Examples
    ///
    /// A plugin whose editor writes its gain as a user's, through a clone of
    /// the parts' handle, so undo records the drag:
    ///
    /// ```
    /// use std::error::Error;
    ///
    /// use seam_host::{Editor, ParentWindow, Parts, Plugin, Size};
    /// # use seam_host::{
    /// #     AuCodes, Block, Category, Layout, PluginInfo, ProcessSetup, Processor, Status,
    /// # };
    /// use seam_params::{Declarer, Float, FloatDecl, Params, UiParams};
    ///
    /// struct GainParams {
    ///     gain: Float,
    /// }
    /// # impl Params for GainParams {
    /// #     fn declare(p: &mut Declarer) -> Self {
    /// #         GainParams { gain: p.add(FloatDecl::new("gain", "Gain", -24.0..=24.0, 0.0)) }
    /// #     }
    /// # }
    /// # struct Level;
    /// # impl Processor for Level {
    /// #     fn process(&mut self, _block: Block<'_, '_>) -> Status {
    /// #         Status::Continue
    /// #     }
    /// # }
    ///
    /// /// What the host's window shows: a panel a toolkit would draw.
    /// struct Panel {
    ///     ui: UiParams,
    ///     gain: Float,
    /// }
    ///
    /// impl Editor for Panel {
    ///     fn size(&self) -> Size {
    ///         Size { width: 320, height: 200 }
    ///     }
    ///
    ///     fn open(&mut self, _parent: &ParentWindow) -> Result<(), Box<dyn Error>> {
    ///         Ok(())
    ///     }
    ///
    ///     fn close(&mut self) {}
    ///     fn set_size(&mut self, _size: Size) {}
    ///     fn set_scale(&mut self, _scale: f64) {}
    ///
    ///     fn tick(&mut self) {
    ///         // A drag the toolkit reported: one undo step, told to the host.
    ///         self.ui.write(&self.gain, -6.0);
    ///     }
    /// }
    ///
    /// struct Gain {
    ///     parts: Parts,
    ///     params: GainParams,
    /// }
    ///
    /// impl Plugin for Gain {
    /// #   const INFO: PluginInfo = PluginInfo {
    /// #       name: "Gain",
    /// #       vendor: "Example",
    /// #       url: "",
    /// #       email: "",
    /// #       description: "",
    /// #       version: "0.1.0",
    /// #       id: "dev.example.gain",
    /// #       vst3_class_id: [0; 16],
    /// #       au_codes: AuCodes { subtype: *b"gain", manufacturer: *b"Exmp" },
    /// #       category: Category::Effect,
    /// #       features: &[],
    /// #       layouts: &[Layout::STEREO],
    /// #       midi_input: false,
    /// #       midi_output: false,
    /// #   };
    ///     type Params = GainParams;
    ///     type Processor = Level;
    ///
    ///     fn new(params: GainParams, parts: Parts) -> Self {
    ///         Gain { parts, params }
    ///     }
    ///
    ///     fn activate(&mut self, _setup: &ProcessSetup) -> Level {
    ///         Level
    ///     }
    ///
    ///     fn editor(&mut self) -> Option<Box<dyn Editor>> {
    ///         let ui = self.parts.ui().clone();
    ///         Some(Box::new(Panel { ui, gain: self.params.gain.clone() }))
    ///     }
    /// }
    ///
    /// // A host asks for a view, and the exporter asks the plugin.
    /// let (params, parts) = Parts::for_plugin::<Gain>()?;
    /// let mut plugin = Gain::new(params, parts);
    /// let mut editor = plugin.editor().ok_or("an editor")?;
    /// assert_eq!(editor.size(), Size { width: 320, height: 200 });
    /// editor.tick();
    /// assert_eq!(plugin.params.gain.get(), -6.0);
    /// # Ok::<(), Box<dyn Error>>(())
    /// ```
    fn editor(&mut self) -> Option<Box<dyn Editor>> {
        None
    }
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
    /// Learned controllers have written the parameters before this call, as
    /// the host's automation has: [`Parts::midi_learn`] says how.
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
