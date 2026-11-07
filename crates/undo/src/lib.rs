//! An undo history in which one user gesture is one step and host
//! automation never records.
//!
//! [`History`] does not listen to writes: it polls once per UI
//! [tick][glossary-tick] ([`History::tick`]). Every change takes a number
//! from the plugin's restore clock, so what one tick finds records in the
//! order it happened. A recorded change is a step, and an
//! [entry][glossary-entry] is what one undo undoes: one step, or every step
//! a [transaction][glossary-transaction] gathered.
//!
//! What records is a user's gesture on an [enrolled][glossary-enrollment]
//! parameter and the plugin's own writes to an enrolled plain value or list.
//! Nothing else does: not host or learned writes, [restores][glossary-restore],
//! program edits and syncs, [replays][glossary-replay], nor a change derived
//! from another, which its source's step covers.
//!
//! A restore clears the history, and a replay stops before its next write
//! once a restore is stored, so an undo never overwrites a loaded value. The
//! history is bounded by the bytes its steps keep alive ([`Bound`]), and a
//! write that changes nothing keeps the redo ([`UndoEq`]).
//!
//! The crate has no window, WebView or host dependency.
//!
//! # Examples
//!
//! Enrol a parameter, make one gesture, and undo it:
//!
//! ```
//! use seam_params::{Kind, ParamDecl, ParamSet};
//! use seam_undo::{Bound, History};
//!
//! let decl = ParamDecl::new("gain", "Gain", Kind::continuous(0.0, 10.0, 1.0, None), 5.0);
//! let set = ParamSet::from_decls([decl])?;
//! let gain = set.id("gain").ok_or("declared above")?;
//! let ui = set.take_ui().ok_or("the first UI handle")?;
//!
//! let history = History::new(&ui, Bound::default())?;
//! history.enroll_param(gain)?;
//!
//! // A drag is one gesture: however many writes it holds, it is one step.
//! let drag = ui.begin(gain);
//! for value in [6.0, 7.0, 8.0] {
//!     drag.write(value);
//! }
//! drop(drag);
//! history.tick(); // once per UI tick
//! assert_eq!(history.len(), 1);
//!
//! assert_eq!(history.undo(), Ok(true));
//! assert_eq!(set.value(gain), 5.0);
//! assert_eq!(history.redo(), Ok(true));
//! assert_eq!(set.value(gain), 8.0);
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```
//!
//! [glossary-tick]: https://github.com/nicvlav/seam/blob/main/docs/glossary.md#tick
//! [glossary-entry]: https://github.com/nicvlav/seam/blob/main/docs/glossary.md#entry-step
//! [glossary-transaction]: https://github.com/nicvlav/seam/blob/main/docs/glossary.md#transaction
//! [glossary-restore]: https://github.com/nicvlav/seam/blob/main/docs/glossary.md#restore
//! [glossary-replay]: https://github.com/nicvlav/seam/blob/main/docs/glossary.md#replay
//! [glossary-enrollment]: https://github.com/nicvlav/seam/blob/main/docs/glossary.md#enrollment

mod enroll;
mod eq;
mod history;
mod list;
mod size;

pub use eq::UndoEq;
pub use history::{History, Transaction};
pub use size::UndoSize;

/// One entry as a history panel lists it ([`History::entries`]).
///
/// # Examples
///
/// ```
/// use seam_params::{Kind, ParamDecl, ParamSetBuilder};
/// use seam_undo::{Bound, History};
///
/// # fn main() -> Result<(), Box<dyn std::error::Error>> {
/// let mut builder = ParamSetBuilder::new();
/// builder.add(ParamDecl::new("gain", "Gain", Kind::continuous(-60.0, 12.0, 0.1, None), 0.0)
///     .with_unit("dB"))?;
/// let set = builder.build();
/// let gain = set.id("gain").ok_or("declared above")?;
/// let ui = set.take_ui().ok_or("the first UI handle")?;
/// let history = History::new(&ui, Bound::default())?;
/// history.enroll_param(gain)?;
///
/// ui.write(gain, -6.0);
/// history.tick();
/// history.undo()?;
///
/// // A history panel lists the applied steps, then the undone ones.
/// let entries = history.entries();
/// assert_eq!(entries.len(), 1);
/// assert!(entries[0].undone);
/// assert_eq!(history.cursor(), 0);
/// # Ok(())
/// # }
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Entry {
    /// The outermost transaction's name, or the enrolled item's name for a
    /// step outside any transaction. May be empty or repeated.
    pub name: String,
    /// Whether it is undone: past the [cursor](History::cursor).
    pub undone: bool,
}

/// How much history is kept.
///
/// When the booked size of the steps exceeds `max_units`, the oldest applied
/// entries are dropped whole, never below `min_entries` of them. A step books
/// the bytes its stored values keep alive ([`UndoSize`], so a vector's heap
/// counts) plus a fixed overhead of 32 units, once, when it is recorded.
///
/// # Examples
///
/// ```
/// use seam_undo::Bound;
///
/// // A plugin whose steps are small can keep more of them.
/// let bound = Bound::default().with_max_units(100_000);
/// assert_eq!(bound.min_entries, 30);
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct Bound {
    /// The soft cap on the total booked size.
    pub max_units: usize,
    /// The fewest applied entries pruning may leave.
    pub min_entries: usize,
}

impl Bound {
    /// Returns the bound with [`max_units`](Self::max_units) set.
    #[must_use]
    pub const fn with_max_units(self, max_units: usize) -> Bound {
        Bound { max_units, ..self }
    }

    /// Returns the bound with [`min_entries`](Self::min_entries) set.
    #[must_use]
    pub const fn with_min_entries(self, min_entries: usize) -> Bound {
        Bound {
            min_entries,
            ..self
        }
    }
}

impl Default for Bound {
    /// 30,000 units and 30 entries: about 750 parameter steps.
    fn default() -> Self {
        Bound {
            max_units: 30_000,
            min_entries: 30,
        }
    }
}

/// Why a history operation was refused.
///
/// A refused operation changed nothing, though a jump out of range first
/// records what changed since the last tick.
///
/// # Examples
///
/// ```
/// use seam_params::{Kind, ParamDecl, ParamSetBuilder};
/// use seam_undo::{Bound, History, HistoryError};
///
/// # fn main() -> Result<(), Box<dyn std::error::Error>> {
/// let mut builder = ParamSetBuilder::new();
/// builder.add(ParamDecl::new("gain", "Gain", Kind::continuous(-60.0, 12.0, 0.1, None), 0.0)
///     .with_unit("dB"))?;
/// let set = builder.build();
/// let ui = set.take_ui().ok_or("the first UI handle")?;
///
/// let history = History::new(&ui, Bound::default())?;
/// // One history per UI handle: make one and clone it.
/// assert_eq!(
///     History::new(&ui, Bound::default()).err(),
///     Some(HistoryError::HistoryExists),
/// );
/// assert_eq!(
///     history.jump(3),
///     Err(HistoryError::OutOfRange { index: 3, len: 0 }),
/// );
/// # Ok(())
/// # }
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum HistoryError {
    /// A replay is applying; undo, redo, jump and clear are refused until it
    /// finishes.
    Replaying,
    /// A user gesture is open: undo, redo and jump wait until it closes, so
    /// a replay never runs inside a drag.
    GestureOpen,
    /// A jump past either end of the history.
    OutOfRange {
        /// The index asked for.
        index: usize,
        /// The number of steps.
        len: usize,
    },
    /// The value is already enrolled; a second enrollment would record every
    /// edit twice.
    AlreadyEnrolled,
    /// The value or list was made on another parameter set than the history's,
    /// so its changes would never record in order.
    OtherClock,
    /// Another history over the same UI handle, or a clone of one, is alive;
    /// two would take each other's gesture records.
    HistoryExists,
}

impl std::fmt::Display for HistoryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            HistoryError::Replaying => write!(f, "a replay is in progress"),
            HistoryError::GestureOpen => write!(f, "a user gesture is open"),
            HistoryError::OutOfRange { index, len } => {
                write!(f, "step {index} is outside a history of {len}")
            }
            HistoryError::AlreadyEnrolled => write!(f, "already enrolled"),
            HistoryError::OtherClock => write!(f, "made with another restore clock"),
            HistoryError::HistoryExists => {
                write!(f, "another history over this UI handle is alive")
            }
        }
    }
}

impl std::error::Error for HistoryError {}
