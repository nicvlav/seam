//! The plugin as every host sees it.
//!
//! A plugin implements [`Plugin`] once, with no knowledge of any format. An
//! exporter, a module behind the cargo feature of its name, projects it into
//! one format, as a thin shell over the core all exporters share.
//!
//! The audio thread gets each block as a [`Block`] in [`Processor::process`]
//! and reads its parameters from the [`ParamSet`](seam_params::ParamSet) that
//! [`SavedState::set`] holds.
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
//! A plugin is one [`Plugin`] impl, which an exporter's macro exports at the
//! root of its crate. [`Plugin::idle`]'s example adds undo, the session and
//! MIDI learn.
//!
//! ```
//! use std::sync::Arc;
//!
//! use seam_host::{
//!     Block, Category, Layout, Plugin, PluginInfo, ProcessSetup, Processor, SavedState, Status,
//! };
//! use seam_params::{AudioParams, Kind, ParamDecl, ParamId, ParamSet};
//! use seam_session::Session;
//!
//! /// The audio-thread half: scales every channel by the gain.
//! struct Level {
//!     params: AudioParams,
//!     gain: ParamId,
//! }
//!
//! impl Processor for Level {
//!     fn process(&mut self, block: Block<'_, '_>) -> Status {
//!         let gain = 10.0_f32.powf(self.params.value(self.gain) / 20.0);
//!         for channel in block.channels.iter_mut() {
//!             for sample in channel.iter_mut() {
//!                 *sample *= gain;
//!             }
//!         }
//!         Status::Continue
//!     }
//! }
//!
//! /// The main-thread half: it holds the saved state, and the parameter set in it.
//! struct Gain {
//!     saved: Arc<SavedState>,
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
//!         au_codes: (*b"gain", *b"Exmp"),
//!         category: Category::Effect,
//!         features: &[],
//!         layouts: &[Layout::STEREO],
//!         midi_input: false,
//!         midi_output: false,
//!     };
//!     const PARAMS: &'static [ParamDecl] = &[
//!         ParamDecl::new("gain", "Gain", Kind::continuous(-24.0, 24.0, 0.0, None), 0.0)
//!             .with_unit("dB"),
//!     ];
//!     type Processor = Level;
//!
//!     fn new() -> Self {
//!         let set = ParamSet::from_decls(Self::PARAMS.iter().copied())
//!             .expect("valid declarations");
//!         Gain { saved: Arc::new(SavedState::new(&set, &Session::new(&set))) }
//!     }
//!
//!     fn saved(&self) -> Arc<SavedState> {
//!         Arc::clone(&self.saved)
//!     }
//!
//!     // Everything the audio thread will touch is built here, on the main thread.
//!     fn activate(&mut self, _setup: &ProcessSetup) -> Level {
//!         let set = self.saved.set();
//!         Level { params: set.audio(), gain: set.id("gain").expect("declared") }
//!     }
//! }
//! ```

// The nightly real-time sanitizer lane marks the audio thread's entry points.
#![cfg_attr(seam_rtsan, feature(sanitize))]

mod audio;
mod info;
mod plugin;
mod state;

// The core every exporter shares: an exporter drives it, and nothing else
// does.
#[allow(unsafe_code, reason = "a host boundary: every block says why it holds")]
#[cfg_attr(
    not(test),
    expect(dead_code, reason = "only an exporter drives the core")
)]
mod boundary;
#[allow(unsafe_code, reason = "a host boundary: every block says why it holds")]
#[expect(dead_code, reason = "only an exporter drives the engine")]
mod engine;
#[allow(unsafe_code, reason = "a host boundary: every block says why it holds")]
#[expect(dead_code, reason = "only an exporter drives the core")]
mod faults;

// Inlined: `Event` and `EventList` are `seam_ump`'s, shown here as this
// crate's, so a plugin needs no `seam_ump` import for its blocks.
#[doc(inline)]
pub use audio::{Block, Event, EventList, MAX_FRAMES, ProcessSetup, Status};
pub use info::{Category, Feature, Layout, PluginInfo};
pub use plugin::{Plugin, Processor, Tail};
pub use state::{LoadError, MAX_STATE_BYTES, STATE_VERSION, SavedState};
