//! Parameters, declared once and owned by Rust.
//!
//! Every parameter has one value [store], in real units. The audio thread, the
//! UI, the host and the save path all read it, so none of them sees a value
//! older than another. Every write names its [origin], and the origin alone
//! decides what else happens: whether the host hears of it, whether it opens
//! a [gesture], whether undo records it, and when the UI shows it.
//!
//! - A plugin declares its parameters in [`Params::declare`]: each typed
//!   declaration, a [`FloatDecl`], [`BoolDecl`] or [`ChoiceDecl`], hands back
//!   its parameter, and [`ParamSet::declare`] freezes them into a
//!   [`ParamSet`], with one store each: an atomic value and change counter.
//!   With the `derive` feature, `#[derive(Params)]` writes `declare` from a
//!   mark on each field, and `#[derive(Choices)]` writes a choice's enum.
//!   [`ParamSetBuilder`] builds a set from a plain table of [`ParamDecl`]s.
//! - One handle per writer: [`AudioParams`] (host and [learned
//!   writes][learned-write]), [`UiParams`] (user [`Gesture`]s, [`Edit`]s, undo
//!   replays) and [`ProgramParams`] ([syncs][sync] and a saved state's
//!   [`Load`], a [restore]).
//! - A typed parameter, [`Float`], [`Bool`] or [`Choice`], is a parameter as
//!   plugin code holds it: it carries its store, and [`get`](Float::get)
//!   returns an `f32`, a `bool` or the plugin's own enum, on any thread.
//!   [`ParamSet::bind`] makes one from a table's key. [`Changes`] tells the
//!   audio thread when any of a group moved.
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
//! A plugin declares each parameter where it holds it. Its processor
//! recomputes only when one moved, and needs no handle to the set, since
//! each parameter carries its store. It is dropped off the audio thread, and
//! a debug build catches a drop there, which could free a store.
//!
//! ```
//! use seam_params::{Bool, BoolDecl, Changes, Declarer, Float, FloatDecl, ParamSet, Params};
//!
//! struct GainParams {
//!     gain: Float,
//!     mute: Bool,
//! }
//!
//! impl Params for GainParams {
//!     fn declare(p: &mut Declarer) -> Self {
//!         GainParams {
//!             gain: p.add(
//!                 FloatDecl::new("gain", "Gain", -24.0..=24.0, 0.0)
//!                     .with_step(0.1)
//!                     .with_unit("dB"),
//!             ),
//!             mute: p.add(BoolDecl::new("mute", "Mute", false)),
//!         }
//!     }
//! }
//!
//! struct Processor {
//!     params: GainParams,
//!     changes: Changes,
//!     factor: f32,
//! }
//!
//! impl Processor {
//!     /// Called once per block, on the audio thread.
//!     fn retune(&mut self) {
//!         let GainParams { gain, mute } = &self.params;
//!         if self.changes.moved([gain, mute]) {
//!             let decibels = gain.get(); // an f32, in dB
//!             let muted = mute.get(); // a bool
//!             self.factor = if muted { 0.0 } else { 10.0_f32.powf(decibels / 20.0) };
//!         }
//!     }
//! }
//!
//! let (set, params) = ParamSet::declare::<GainParams>()?;
//! let mut processor = Processor {
//!     params,
//!     changes: Changes::new(),
//!     factor: 1.0,
//! };
//!
//! // The host automates the gain to the top of its range (0..1 is its scale).
//! set.audio().host_write(processor.params.gain.id(), 1.0);
//! processor.retune();
//! assert!((processor.factor - 15.848_932).abs() < 1e-3); // +24 dB
//! # Ok::<(), seam_params::DeclError>(())
//! ```
//!
//! Invariants: a value never leaves its legal set (writes snap, and a value
//! that is not finite never reaches a store). A [`Reading`]'s value is never
//! older than its change count. The audio thread's writes never lock or
//! allocate.

// The nightly real-time sanitizer lane marks the calls the audio thread makes.
#![cfg_attr(seam_rtsan, feature(sanitize))]
// docs.rs shows which feature each gated item needs.
#![cfg_attr(docsrs, feature(doc_cfg))]

#[cfg(not(target_has_atomic = "64"))]
compile_error!(
    "seam-params needs 64-bit atomics: a parameter's value and origin are stored as one 64-bit word"
);

pub mod audio_thread;
mod changes;
mod clock;
pub mod decl;
mod declare;
mod events;
mod host_events;
mod host_id;
mod list;
mod meta;
mod origin;
mod param;
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
pub use declare::{
    BoolDecl, ChoiceDecl, Declaration, Declared, Declarer, FloatDecl, Params,
    discriminant_is_position, key_is_new, labels_are_valid,
};
pub use events::{GestureRecord, HostEvent};
pub use host_events::HostEventDrain;
pub use host_id::{RESERVED_HOST_IDS, host_id};
pub use list::{Derived, List, ListCommit, ListTracker, TrackedList};
pub use meta::fit_text;
pub use origin::{Origin, Reading, UnknownOrigin};
pub use param::{AnyParam, Bind, BindError, Bool, Choice, Choices, Float, Writable};
pub use program::{Load, ProgramParams};
pub use set::{AudioParams, DeclError, MAX_KEY_LEN, ParamSet, ParamSetBuilder};
pub use ui::{Edit, Gesture, GestureRecords, UiParams};
pub use value::{Commit, TrackedRef, Tracker, Value, ValueGesture};
pub use watch::Watch;

/// Writes a struct's [`Params::declare`] from a `#[param(...)]` mark on each
/// field.
///
/// Each marked field is one parameter, and its type says which declaration
/// makes it, through [`Declared`]: a [`Float`] field is a [`FloatDecl`], a
/// [`Bool`] a [`BoolDecl`] and a [`Choice<E>`] a [`ChoiceDecl<E>`]. Inside
/// the mark:
///
/// - the leading items are the arguments of the declaration's `new`, as
///   written: the key, the name, a float's range, and the default;
/// - `name = value` is its `with_name(value)`: `step`, `skew_centre`, `unit`
///   and `group`;
/// - a lone lowercase word is its `as_word()`: `bypass`. `true` and `false`
///   are values, and a constant is written in capitals.
///
/// A field with no mark is a struct of parameters itself, and declares
/// itself through its own [`Params`], derived or written by hand.
///
/// The derive writes what a person would: one `impl Params` that adds each
/// field in order, through [`Declarer::add`]. Each declaration is a named
/// constant that ends in [`checked`](FloatDecl::checked), so a plain `cargo
/// check` refuses a key, a range or a label no parameter could have, and a
/// key an earlier field of the struct has, on the mark. A key that two
/// structs share is refused when the set is built, as by hand.
///
/// Arrays, optional parameters and generic structs are not derived: write
/// `declare` by hand for those, as for a set whose parameters come from a
/// table.
///
/// rustfmt leaves a mark as it is typed, so a long one is wrapped by hand.
///
/// # Examples
///
/// ```
/// use seam_params::{Bool, Choice, Choices, Float, ParamSet, Params};
///
/// #[derive(Debug, Clone, Copy, PartialEq, Choices)]
/// enum Mode {
///     Clean,
///     Warm,
/// }
///
/// #[derive(Params)]
/// struct Band {
///     #[param("low.freq", "Frequency", 20.0..=500.0, 100.0, unit = "Hz", group = "Low")]
///     freq: Float,
/// }
///
/// #[derive(Params)]
/// struct Drive {
///     #[param(
///         "gain", "Gain", -60.0..=12.0, 0.0,
///         step = 0.1, unit = "dB",
///     )]
///     gain: Float,
///     #[param("mode", "Mode", Mode::Warm)]
///     mode: Choice<Mode>,
///     #[param("bypass", "Bypass", false, bypass)]
///     bypass: Bool,
///     low: Band,
/// }
///
/// let (set, drive) = ParamSet::declare::<Drive>()?;
/// assert_eq!(drive.gain.get(), 0.0);
/// assert_eq!(drive.mode.get(), Mode::Warm);
/// assert_eq!(set.bypass(), Some(drive.bypass.id()));
/// assert_eq!(set.decl(drive.low.freq.id()).group, "Low");
/// # Ok::<(), seam_params::DeclError>(())
/// ```
#[cfg(feature = "derive")]
#[cfg_attr(docsrs, doc(cfg(feature = "derive")))]
pub use seam_macros::Params;

/// Writes [`Choices`](trait@Choices) for a fieldless enum: its labels, and
/// its two mappings, in the order the variants are written.
///
/// A variant's label is its name, unless a `#[choice("...")]` mark on it
/// gives one. A label is shown and never saved, so renaming a variant changes
/// only the text a host shows. [`from_index`](Choices::from_index) gives the
/// last variant for an index past the end.
///
/// The order of the variants is the save format. A choice is saved as its
/// index, so a variant added at the end keeps saved projects right, but it
/// moves every position a host sees: automation a host recorded against the
/// shorter list plays other values. A variant moved, or inserted before the
/// last, loads an older project into the wrong value unless the plugin's
/// `SCHEMA_VERSION` goes up and its `migrate` maps the old index to the new.
/// The derive keeps the labels and both mappings in one order; it cannot see
/// the order a saved project was written in.
///
/// While compiling, the derive refuses a struct, a union, a generic enum, an
/// enum of fewer than two variants and a variant with fields, each on the
/// author's own line. A plain `cargo check` refuses, on the enum, labels a
/// choice could not have: an empty one, one with whitespace at an end, and
/// two that differ only in ASCII case. A discriminant the author writes must
/// be the variant's position, and any other value is refused on that
/// discriminant: an enum whose discriminants mean something else writes
/// [`Choices`](trait@Choices) by hand.
///
/// # Examples
///
/// ```
/// use seam_params::{Choice, Choices, ParamSet, Params};
///
/// #[derive(Debug, Clone, Copy, PartialEq, Choices)]
/// enum Slope {
///     #[choice("12 dB/oct")]
///     Gentle,
///     #[choice("24 dB/oct")]
///     Steep,
///     Brick,
/// }
///
/// #[derive(Params)]
/// struct Filter {
///     #[param("slope", "Slope", Slope::Steep)]
///     slope: Choice<Slope>,
/// }
///
/// assert_eq!(Slope::LABELS, ["12 dB/oct", "24 dB/oct", "Brick"]);
/// assert_eq!(Slope::Steep.index(), 1);
/// assert_eq!(Slope::from_index(usize::MAX), Slope::Brick);
///
/// let (set, filter) = ParamSet::declare::<Filter>()?;
/// assert_eq!(filter.slope.get(), Slope::Steep);
/// assert_eq!(set.decl(filter.slope.id()).name, "Slope");
/// # Ok::<(), seam_params::DeclError>(())
/// ```
#[cfg(feature = "derive")]
#[cfg_attr(docsrs, doc(cfg(feature = "derive")))]
pub use seam_macros::Choices;
