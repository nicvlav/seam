//! Parameters, declared once and owned by Rust.
//!
//! Every parameter has one value [store], in real units. The audio thread, the
//! UI, the host and the save path all read it, so none of them sees a value
//! older than another. Every write names its [origin], and the origin alone
//! decides what else happens: whether the host hears of it, whether it opens
//! a [gesture], whether undo records it, and when the UI shows it.
//!
//! - [`ParamSetBuilder`] checks [`ParamDecl`]s once and freezes them into a
//!   [`ParamSet`], with one atomic value cell and change counter each.
//! - One handle per writer: [`AudioParams`] (host and [learned
//!   writes][learned-write]), [`UiParams`] (user [`Gesture`]s, [`Edit`]s, undo
//!   replays) and [`ProgramParams`] ([syncs][sync] and a saved state's
//!   [`Load`], a [restore]).
//! - A typed handle, [`Float`], [`Bool`] or [`Choice`], is a parameter as
//!   plugin code holds it: [`ParamSet::bind`] checks it against the
//!   declaration once, and each read returns an `f32`, a `bool` or the
//!   plugin's own enum. [`Changes`] tells the audio thread when any of a group
//!   moved.
//! - [`Value`] and [`List`] hold plugin state that is not a parameter, and
//!   undo enrols them.
//! - A [`Watch`] (a [watch]) shows one parameter on a [surface]; a
//!   [`HostEventDrain`] [drains][drain] to the host what [edits][edit] changed.
//!
//! The [glossary] defines the linked terms.
//!
//! [store]: https://github.com/nicvlav/seam/blob/main/docs/glossary.md#store
//! [origin]: https://github.com/nicvlav/seam/blob/main/docs/glossary.md#origin
//! [gesture]: https://github.com/nicvlav/seam/blob/main/docs/glossary.md#gesture
//! [learned-write]: https://github.com/nicvlav/seam/blob/main/docs/glossary.md#learned-write
//! [sync]: https://github.com/nicvlav/seam/blob/main/docs/glossary.md#sync
//! [restore]: https://github.com/nicvlav/seam/blob/main/docs/glossary.md#restore
//! [surface]: https://github.com/nicvlav/seam/blob/main/docs/glossary.md#surface
//! [watch]: https://github.com/nicvlav/seam/blob/main/docs/glossary.md#watch
//! [edit]: https://github.com/nicvlav/seam/blob/main/docs/glossary.md#edit
//! [drain]: https://github.com/nicvlav/seam/blob/main/docs/glossary.md#drain
//! [glossary]: https://github.com/nicvlav/seam/blob/main/docs/glossary.md
//!
//! # Examples
//!
//! A processor holds the audio thread's handle and a typed handle per
//! parameter, bound once when the plugin is built, and recomputes only when one
//! moved. It holds [`AudioParams`], not an `Arc<ParamSet>`, so a debug build
//! catches a drop on the audio thread that would free the set.
//!
//! ```
//! use seam_params::{AudioParams, Bool, Changes, Float, Handle, Kind, ParamDecl, ParamSet};
//!
//! struct Processor {
//!     audio: AudioParams,
//!     gain: Float,
//!     mute: Bool,
//!     changes: Changes<2>,
//!     factor: f32,
//! }
//!
//! impl Processor {
//!     /// Called once per block, on the audio thread.
//!     fn retune(&mut self) {
//!         if self.changes.moved(&self.audio) {
//!             let decibels = self.audio.value(self.gain); // an f32, in dB
//!             let muted = self.audio.value(self.mute); // a bool
//!             self.factor = if muted { 0.0 } else { 10.0_f32.powf(decibels / 20.0) };
//!         }
//!     }
//! }
//!
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! const PARAMS: &[ParamDecl] = &[
//!     ParamDecl::new("gain", "Gain", Kind::continuous(-24.0, 24.0, 0.1, None), 0.0)
//!         .with_unit("dB"),
//!     ParamDecl::new("mute", "Mute", Kind::Bool, 0.0),
//! ];
//! let params = ParamSet::from_decls(PARAMS.iter().copied())?;
//! let gain: Float = params.bind("gain")?;
//! let mute: Bool = params.bind("mute")?;
//! let mut processor = Processor {
//!     audio: params.audio(),
//!     gain,
//!     mute,
//!     changes: Changes::new([gain.id(), mute.id()]),
//!     factor: 1.0,
//! };
//!
//! // The host automates the gain to the top of its range (0..1 is its scale).
//! processor.audio.host_write(gain.id(), 1.0);
//! processor.retune();
//! assert!((processor.factor - 15.848_932).abs() < 1e-3); // +24 dB
//! # Ok(())
//! # }
//! ```
//!
//! Invariants: a value never leaves its legal set (writes snap, and a value
//! that is not finite never reaches a store). A [`Reading`]'s value is never
//! older than its change count. The audio thread's writes never lock or
//! allocate.

// The nightly real-time sanitizer lane marks the calls the audio thread makes.
#![cfg_attr(seam_rtsan, feature(sanitize))]

#[cfg(not(target_has_atomic = "64"))]
compile_error!(
    "seam-params needs 64-bit atomics: a parameter's value and origin are stored as one 64-bit word"
);

pub mod audio_thread;
mod changes;
mod clock;
pub mod decl;
mod events;
mod handle;
mod host_events;
mod host_id;
mod list;
mod meta;
mod origin;
mod program;
mod set;
mod ui;
mod value;
mod watch;

pub use changes::Changes;
pub use clock::{
    ApplyingGuard, LoadHold, RestoreClock, RestoreReader, RestoreStamp, RestoreWriter, Stamped,
};
// The module is public too, so rustdoc would list these as bare re-exports.
#[doc(inline)]
pub use decl::{Kind, ParamDecl, ParamId, Range, RangeError};
pub use events::{GestureRecord, HostEvent};
pub use handle::{Bind, BindError, Bool, Choice, Choices, Float, Handle};
pub use host_events::HostEventDrain;
pub use host_id::{RESERVED_HOST_IDS, host_id};
pub use list::{Derived, List, ListCommit, ListTracker, TrackedList};
pub use meta::fit_text;
pub use origin::{Origin, Reading, UnknownOrigin};
pub use program::{Load, ProgramParams};
pub use set::{AudioParams, DeclError, MAX_KEY_LEN, ParamSet, ParamSetBuilder};
pub use ui::{Edit, Gesture, GestureRecords, UiParams};
pub use value::{Commit, TrackedRef, Tracker, Value, ValueGesture};
pub use watch::Watch;
