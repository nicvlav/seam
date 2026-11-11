//! One parameter's cell: its value word and change count, which every read
//! touches, then its id and declaration, which a typed read never does.

#[cfg(loom)]
use loom::sync::atomic::{AtomicU64, Ordering};
#[cfg(not(loom))]
use std::sync::atomic::{AtomicU64, Ordering};

use super::value_cell::{Packed, ValueCell};
use crate::{Origin, ParamDecl, ParamId, Reading};

/// One parameter's store, shared by its set and by whatever holds it.
///
/// `repr(C)` keeps the fields in this order. The value word and the count sit
/// in the first 16 bytes, so a read that compares a count and loads the value
/// touches one cache line and never the declaration behind them.
#[repr(C)]
#[derive(Debug)]
pub(crate) struct ParamCell {
    value: ValueCell,
    /// Moved by every write after its store; see
    /// [`store_then_publish`](Self::store_then_publish).
    changes: AtomicU64,
    id: ParamId,
    decl: ParamDecl,
}

impl ParamCell {
    /// A cell at its declaration's snapped default, as a restore's write.
    pub(crate) fn new(id: ParamId, decl: ParamDecl) -> Self {
        ParamCell {
            value: ValueCell::new(Packed::new(decl.default_value(), Origin::Restore)),
            changes: AtomicU64::new(0),
            id,
            decl,
        }
    }

    pub(crate) fn id(&self) -> ParamId {
        self.id
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
