//! The plugin as every host sees it.
//!
//! A plugin implements [`Plugin`] once, with no knowledge of any format. An
//! exporter, a module behind the cargo feature of its name, projects it into
//! one format, as a thin shell over the core all exporters share: the `vst3`
//! module natively, and the `cabi` module as a C ABI plus a generated header.
//! One generic JUCE adapter turns that into AU, AUv3, AAX and hosting.
//!
//! Seam builds each instance's [`Parts`], its parameter set, session and
//! saved state, from the plugin's declarations, and hands them to
//! [`Plugin::new`]. The audio thread gets each block as a [`Block`] in
//! [`Processor::process`], and reads the parameters the plugin handed its
//! processor.
//!
//! What a host reads before creating an instance is [`PluginInfo`].
//! [`SavedState`] is the saved block, the plugin's parameters and session as
//! one JSON object, saved and loaded from any thread but audio.
//!
//! Invariants:
//!
//! - The main thread owns the plugin ([`Plugin`] says which thread that
//!   is) and builds its [`Processor`] at activation, allocating everything
//!   the audio thread will touch up front. The audio thread then owns the
//!   processor and never allocates, locks or makes a system call, except
//!   while a panic unwinds.
//! - `unsafe` is denied workspace-wide. Only the exporters and the
//!   `boundary`, `engine` and `faults` modules they share allow it, with a
//!   `// SAFETY:` comment on every block.
//!
//! # Examples
//!
//! A plugin is one [`Plugin`] impl and one export macro at the root of its
//! crate: `export_vst3!` for a VST3 module, or `export_cabi!` for the JUCE
//! adapter's static library. [`Plugin::idle`]'s example adds undo and MIDI
//! learn.
//!
//! ```no_run
//! use seam_host::{
//!     AuCodes, Block, Category, Layout, Parts, Plugin, PluginInfo, ProcessSetup, Processor,
//!     Status,
//! };
//! use seam_params::{Declarer, Float, FloatDecl, Params};
//!
//! /// The parameters, each declared where it is held.
//! #[derive(Clone)]
//! struct GainParams {
//!     gain: Float,
//! }
//!
//! impl Params for GainParams {
//!     fn declare(p: &mut Declarer) -> Self {
//!         GainParams {
//!             gain: p.add(FloatDecl::new("gain", "Gain", -24.0..=24.0, 0.0).with_unit("dB")),
//!         }
//!     }
//! }
//!
//! /// The audio-thread half: scales every channel by the gain.
//! struct Level(GainParams);
//!
//! impl Processor for Level {
//!     fn process(&mut self, block: Block<'_, '_>) -> Status {
//!         let gain = 10.0_f32.powf(self.0.gain.get() / 20.0);
//!         for channel in block.channels.iter_mut() {
//!             for sample in channel.iter_mut() {
//!                 *sample *= gain;
//!             }
//!         }
//!         Status::Continue
//!     }
//! }
//!
//! /// The main-thread half: it keeps the parameters Seam declared for it.
//! struct Gain {
//!     params: GainParams,
//! }
//!
//! impl Plugin for Gain {
//!     const INFO: PluginInfo = PluginInfo {
//!         name: "Gain",
//!         vendor: "Example",
//!         url: "",
//!         email: "",
//!         description: "",
//!         version: "0.1.0",
//!         id: "dev.example.gain",
//!         // A real plugin needs its own fixed, unique id here.
//!         vst3_class_id: [0; 16],
//!         au_codes: AuCodes { subtype: *b"gain", manufacturer: *b"Exmp" },
//!         category: Category::Effect,
//!         features: &[],
//!         layouts: &[Layout::STEREO],
//!         midi_input: false,
//!         midi_output: false,
//!     };
//!     type Params = GainParams;
//!     type Processor = Level;
//!
//!     fn new(params: GainParams, _parts: Parts) -> Self {
//!         Gain { params }
//!     }
//!
//!     // Everything the audio thread will touch is built here, on the main thread.
//!     fn activate(&mut self, _setup: &ProcessSetup) -> Level {
//!         Level(self.params.clone())
//!     }
//! }
//!
//! // At the root of the plugin's `cdylib` crate.
//! # #[cfg(all(feature = "vst3", any(windows, target_os = "macos")))]
//! seam_host::export_vst3!(Gain);
//! ```

// The nightly real-time sanitizer lane marks the audio thread's entry points.
#![cfg_attr(seam_rtsan, feature(sanitize))]
// docs.rs shows which feature each gated item needs.
#![cfg_attr(docsrs, feature(doc_cfg))]

mod audio;
mod info;
mod parts;
mod plugin;
mod state;

// The core every exporter shares: an exporter drives it, and nothing else
// does.
#[allow(unsafe_code, reason = "a host boundary: every block says why it holds")]
#[cfg_attr(
    not(any(feature = "vst3", feature = "cabi", test)),
    expect(dead_code, reason = "only an exporter drives the core")
)]
mod boundary;
#[cfg(feature = "cabi")]
#[cfg_attr(docsrs, doc(cfg(feature = "cabi")))]
#[allow(unsafe_code, reason = "a host boundary: every block says why it holds")]
pub mod cabi;
#[allow(unsafe_code, reason = "a host boundary: every block says why it holds")]
#[cfg_attr(
    not(any(feature = "vst3", feature = "cabi")),
    expect(dead_code, reason = "only an exporter drives the engine")
)]
mod engine;
#[allow(unsafe_code, reason = "a host boundary: every block says why it holds")]
#[cfg_attr(
    not(any(feature = "vst3", feature = "cabi")),
    expect(dead_code, reason = "only an exporter drives the core")
)]
mod faults;
#[cfg(feature = "vst3")]
#[cfg_attr(docsrs, doc(cfg(feature = "vst3")))]
#[allow(unsafe_code, reason = "a host boundary: every block says why it holds")]
pub mod vst3;

// Inlined: `Event` and `EventList` are `seam_ump`'s, shown here as this
// crate's, so a plugin needs no `seam_ump` import for its blocks.
#[doc(inline)]
pub use audio::{Block, Event, EventList, MAX_FRAMES, ProcessSetup, Status};
#[cfg(any(feature = "vst3", feature = "cabi"))]
pub use faults::boundary_faults;
pub use info::{AuCodes, Category, Feature, Layout, PluginInfo};
pub use parts::{Driver, Maker, Parts};
pub use plugin::{Plugin, Processor, Tail};
pub use state::{LoadError, MAX_STATE_BYTES, STATE_VERSION, SavedState};
