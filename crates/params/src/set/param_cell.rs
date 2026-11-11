//! One parameter's cell: its value word and change count, which every read
//! touches, then its id and declaration, which a typed read never does.
//!
//! A cell is made when its parameter is declared, before its set orders the
//! parameters, so its id is filled in once, when the set is built.

#[cfg(loom)]
use loom::sync::atomic::{AtomicU64, Ordering};
use std::sync::OnceLock;
use std::sync::atomic::AtomicU64 as SerialCounter;
#[cfg(not(loom))]
use std::sync::atomic::{AtomicU64, Ordering};

use super::value_cell::{Packed, ValueCell};
use crate::{Origin, ParamDecl, ParamId, Reading};

/// The serial the next cell takes. Not a modelled atomic: it only hands out
/// numbers, and orders nothing.
static SERIALS: SerialCounter = SerialCounter::new(0);

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
    /// Set once, by [`place`](Self::place), when the set is built.
    id: OnceLock<ParamId>,
    decl: ParamDecl,
}

impl ParamCell {
    /// A cell at its declaration's snapped default, as a restore's write,
    /// with no id until a set places it.
    pub(crate) fn new(decl: ParamDecl) -> Self {
        ParamCell {
            value: ValueCell::new(Packed::new(decl.default_value(), Origin::Restore)),
            changes: AtomicU64::new(0),
            serial: SERIALS.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
            id: OnceLock::new(),
            decl,
        }
    }

    /// Gives the cell its place in the set being built. A builder owns its
    /// cells and is consumed by the build, so no cell is placed twice.
    pub(crate) fn place(&self, id: ParamId) {
        let first = self.id.set(id).is_ok();
        debug_assert!(first, "a parameter was placed in two sets");
    }

    pub(crate) fn serial(&self) -> u64 {
        self.serial
    }

    /// Its place in its set.
    ///
    /// Asking before the set is built is the caller's bug: a debug build
    /// panics, and a release build returns an id past any set's end.
    pub(crate) fn id(&self) -> ParamId {
        debug_assert!(
            self.id.get().is_some(),
            "a parameter has no id until its set is built: ask for it after ParamSet::declare returns"
        );
        self.placed().unwrap_or(ParamId::from_index(usize::MAX))
    }

    /// Its place in its set, or `None` before the set is built and for a
    /// parameter no set took.
    pub(crate) fn placed(&self) -> Option<ParamId> {
        self.id.get().copied()
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
