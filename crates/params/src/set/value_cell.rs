//! A parameter's value cell: its value and the origin of the write that
//! stored it, packed in one atomic word, so a racing writer can never split
//! them.

#[cfg(loom)]
use loom::sync::atomic::{AtomicU64, Ordering};
#[cfg(not(loom))]
use std::sync::atomic::{AtomicU64, Ordering};

use crate::Origin;

/// A value and its origin, as the cell holds them: the value's bits in the
/// low 32, the origin's number in the byte above them. Only [`new`](Self::new)
/// makes one, so every word names an origin.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct Packed(u64);

impl Packed {
    /// Where the origin's number starts.
    const ORIGIN_SHIFT: u32 = 32;

    pub(super) fn new(value: f32, origin: Origin) -> Packed {
        Packed((u64::from(origin as u8) << Self::ORIGIN_SHIFT) | u64::from(value.to_bits()))
    }

    pub(super) fn value(self) -> f32 {
        // The cast keeps the low 32 bits, which are the value's.
        f32::from_bits(self.0 as u32)
    }

    pub(super) fn origin(self) -> Origin {
        // `new` wrote an origin's number here, and `Origin::ALL` lists every
        // origin, so the fallback is never taken.
        Origin::try_from((self.0 >> Self::ORIGIN_SHIFT) as u8).unwrap_or(Origin::Restore)
    }
}

/// One parameter's cell, which stores and loads a [`Packed`] word whole.
pub(super) struct ValueCell(AtomicU64);

impl std::fmt::Debug for ValueCell {
    /// The value and origin the cell holds now, rather than the packed word.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let word = self.load(Ordering::Relaxed);
        f.debug_struct("ValueCell")
            .field("value", &word.value())
            .field("origin", &word.origin())
            .finish()
    }
}

impl ValueCell {
    pub(super) fn new(word: Packed) -> ValueCell {
        ValueCell(AtomicU64::new(word.0))
    }

    pub(super) fn load(&self, order: Ordering) -> Packed {
        Packed(self.0.load(order))
    }

    pub(super) fn store(&self, word: Packed, order: Ordering) {
        self.0.store(word.0, order);
    }
}

#[cfg(all(test, not(loom)))]
mod tests {
    use super::*;

    #[test]
    fn a_word_gives_back_the_value_and_the_origin_it_was_made_with() {
        // Each sign, the extremes and a value with its low bits set, so a
        // field that overlapped the other would show.
        let values = [0.0, -1.5, f32::MIN, f32::MAX, f32::from_bits(0x8000_0001)];
        for origin in Origin::ALL {
            for value in values {
                let word = Packed::new(value, origin);
                assert_eq!(
                    word.value().to_bits(),
                    value.to_bits(),
                    "{value} {origin:?}"
                );
                assert_eq!(word.origin(), origin, "{value} {origin:?}");
            }
        }
    }
}
