//! One parameter's cell: its value word and change count, which every read
//! touches, then its id and declaration, which a typed read never does.
//!
//! A cell is made when its parameter is declared, before its set orders the
//! parameters, so its id is filled in once, when the set is built.

#[cfg(loom)]
use loom::sync::atomic::{AtomicU64, Ordering};
use std::sync::atomic::AtomicU64 as SerialCounter;
use std::sync::atomic::AtomicUsize as HolderCount;
use std::sync::atomic::{AtomicU32 as IdWord, Ordering as IdOrdering};
#[cfg(not(loom))]
use std::sync::atomic::{AtomicU64, Ordering};

use super::value_cell::{Packed, ValueCell};
use crate::{Origin, ParamDecl, ParamId, Reading};

/// The serial the next cell takes. Not a modelled atomic: it only hands out
/// numbers, and orders nothing.
static SERIALS: SerialCounter = SerialCounter::new(0);

/// The id word of a cell no set has placed: past any set's end, since a set
/// never holds 2^32 parameters.
///
/// The word is not a modelled atomic either, and its loads are relaxed: it is
/// written once, while the set is built, before the set or any parameter in
/// it can reach another thread, and whatever hands them over orders that
/// write before the other thread's reads.
const UNPLACED: u32 = u32::MAX;

/// The instance word of a cell a set lists, or will once it is built. A set
/// numbers its instance from 1.
pub(crate) const LISTED: u32 = 0;

/// One parameter's store, shared by its set and by whatever holds it.
///
/// `repr(C)` keeps the fields in this order: what a block reads (the value
/// word, the count and the serial) sits together at the front, and the
/// declaration, which no block read touches, behind it.
#[repr(C)]
#[derive(Debug)]
pub(crate) struct ParamCell {
    value: ValueCell,
    /// Moved by every write after its store; see
    /// [`store_then_publish`](Self::store_then_publish).
    changes: AtomicU64,
    /// Which cell this is, for as long as the process runs: unlike an
    /// address, a serial is never handed out twice.
    serial: u64,
    /// Its id's word, [`UNPLACED`] until [`place`](Self::place) sets it once,
    /// when the set is built. Read on every call of a typed parameter's `id`,
    /// so it is one plain load.
    id: IdWord,
    /// [`LISTED`], or for a parameter made at run time the number of the
    /// instance that made it, so another instance's handles refuse it. In
    /// the padding the id word leaves.
    instance: u32,
    /// How many typed parameters (`Float`, `Bool`, `Choice`) hold it. What
    /// Seam holds itself, a learn route, an undo step, an open gesture or
    /// the save's list, does not count, so learn and the save can tell when
    /// the plugin has let a parameter made at run time go.
    ///
    /// It orders nothing, so every access is relaxed. For a cell made at run
    /// time, zero is final: a typed parameter is made from the store only
    /// once, when it is declared, and every later one is a clone, counted
    /// before the parameter it was cloned from can drop. A listed cell's
    /// count can rise from zero again, through `ParamSet::bind`, and nothing
    /// reads a listed cell's.
    holders: HolderCount,
    decl: ParamDecl,
}

impl ParamCell {
    /// A cell at its declaration's snapped default, as a restore's write,
    /// with no id until a set places it.
    pub(crate) fn new(decl: ParamDecl) -> Self {
        let value = decl.default_value();
        Self::holding(decl, value, LISTED)
    }

    /// A cell for a parameter made at run time by instance `instance`, born
    /// holding `value`, which must be legal for `decl`, as a restore's write.
    /// No set ever places it.
    pub(crate) fn run_time(decl: ParamDecl, value: f32, instance: u32) -> Self {
        debug_assert!(instance != LISTED, "a run-time cell names its instance");
        Self::holding(decl, value, instance)
    }

    fn holding(decl: ParamDecl, value: f32, instance: u32) -> Self {
        ParamCell {
            value: ValueCell::new(Packed::new(value, Origin::Restore)),
            changes: AtomicU64::new(0),
            serial: SERIALS.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
            id: IdWord::new(UNPLACED),
            instance,
            holders: HolderCount::new(0),
            decl,
        }
    }

    /// One more typed parameter holds it.
    pub(crate) fn hold(&self) {
        self.holders.fetch_add(1, IdOrdering::Relaxed);
    }

    /// One typed parameter let it go.
    pub(crate) fn let_go(&self) {
        self.holders.fetch_sub(1, IdOrdering::Relaxed);
    }

    /// Whether a typed parameter still holds it.
    pub(crate) fn held(&self) -> bool {
        self.holders.load(IdOrdering::Relaxed) > 0
    }

    /// The instance a parameter made at run time belongs to, or [`LISTED`].
    pub(crate) fn instance(&self) -> u32 {
        self.instance
    }

    /// Whether it was made at run time, in no set.
    pub(crate) fn is_run_time(&self) -> bool {
        self.instance != LISTED
    }

    /// Writes a learned controller's 0 to 1 as
    /// [`AudioParams::learned_write`](crate::AudioParams::learned_write)
    /// does: what [`ParamDecl::learned_value`] makes of it, and NaN writes
    /// nothing. Never allocates, locks or frees: audio thread.
    pub(crate) fn learned_write(&self, normalized: f64) {
        if let Some(value) = self.decl.learned_value(normalized) {
            self.store_then_publish(value, Origin::Learn, || {});
        }
    }

    /// Gives the cell its place in the set being built. A builder owns its
    /// cells and is consumed by the build, so no cell is placed twice.
    pub(crate) fn place(&self, id: ParamId) {
        let before = self.id.swap(id.raw(), IdOrdering::Relaxed);
        debug_assert!(before == UNPLACED, "a parameter was placed in two sets");
    }

    pub(crate) fn serial(&self) -> u64 {
        self.serial
    }

    /// Its place in its set.
    ///
    /// Asking for a parameter made at run time, or before the set is built,
    /// is the caller's bug: a debug build panics, and a release build returns
    /// an id past any set's end.
    pub(crate) fn id(&self) -> ParamId {
        let raw = self.id.load(IdOrdering::Relaxed);
        debug_assert!(
            !self.is_run_time(),
            "a parameter made at run time has no id: pass the parameter itself"
        );
        debug_assert!(
            raw != UNPLACED,
            "a parameter has no id until its set is built: ask for it after ParamSet::declare returns"
        );
        ParamId::from_raw(raw)
    }

    /// Its place in its set, or `None` before the set is built and for a
    /// parameter no set took.
    pub(crate) fn placed(&self) -> Option<ParamId> {
        let raw = self.id.load(IdOrdering::Relaxed);
        (raw != UNPLACED).then_some(ParamId::from_raw(raw))
    }

    pub(crate) fn decl(&self) -> &ParamDecl {
        &self.decl
    }

    /// The value now, in real units.
    pub(crate) fn value(&self) -> f32 {
        self.value.load(Ordering::Relaxed).value()
    }

    /// How many writes have reached it.
    pub(crate) fn changes(&self) -> u64 {
        // Acquire pairs with the release in `store_then_publish`.
        self.changes.load(Ordering::Acquire)
    }

    /// The count, then the value and its origin from one word.
    pub(crate) fn read(&self) -> Reading<f32> {
        // The count first, as `store_then_publish` explains.
        let changes = self.changes();
        let word = self.value.load(Ordering::Relaxed);
        Reading {
            changes,
            value: word.value(),
            origin: word.origin(),
        }
    }

    /// Every write, in its one order: the value and its origin in one store,
    /// then `between`, then the publish. The value is legal.
    ///
    /// The publish is the count's release, which pairs with the acquire in
    /// [`changes`](Self::changes): a reader that loads the count and then the
    /// value never sees a value older than the count. The store comes before
    /// `between` because `between` may tell the host, which can read the
    /// value back at once; nothing depends on `between` coming before the
    /// publish. A [`value`](Self::value) call may see the write sooner.
    pub(crate) fn store_then_publish(&self, value: f32, origin: Origin, between: impl FnOnce()) {
        // Relaxed: a release after this in program order makes the word
        // visible to a reader that acquires it. That is the count's below for
        // a watch, and for the host's drain the release `between` makes when
        // it tells the host.
        self.value
            .store(Packed::new(value, origin), Ordering::Relaxed);
        between();
        self.changes.fetch_add(1, Ordering::Release);
    }
}
