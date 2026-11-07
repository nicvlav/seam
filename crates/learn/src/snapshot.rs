//! The published table, from the UI thread to the audio thread.
//!
//! A `triple_buffer` carries it, so this crate writes no `unsafe`. Checked
//! in its 9.0.0 source:
//!
//! - The audio thread's read (`Output::read`) is `update` and then a
//!   reference to the reader's own buffer. `update` loads a dirty bit
//!   (relaxed) and, only when it is set, swaps buffer indices with one
//!   `swap(AcqRel)`. It never waits, locks, allocates or frees. The
//!   swap gives the reader exclusive use of the newest table, which the
//!   writer never touches again until a later swap hands it back.
//! - The UI thread's publish (`Input::write`) assigns the new table into the
//!   writer's own buffer. The stale table that buffer held drops there, on the
//!   UI thread. That table is one published two publishes back, or one the
//!   reader let go. A replaced table therefore lives until a later publish:
//!   three tables exist between publishes, four during one.
//! - The buffers are freed when both ends have dropped, by whichever drops
//!   last. [`Reader`] is held by the audio side's handle, which must be
//!   dropped off the audio thread.
//!
//! The reader's swap and the writer's swap are read-modify-writes on one
//! atomic, so one of them always sees the other.
//!
//! The version is pinned exactly, since all of this rests on its source.
//! Should a later one break it, the fallback is a hand-written table with a
//! reader count and a `SeqCst` fence on each side, the form `loom` can
//! check. This module would then allow `unsafe` under the rules the
//! exporters follow, with a miri job.

// Audio-path code: a panic here ends a host, so indexing, unwrap, expect,
// panic and unchecked arithmetic are refused, and a site that is provably
// safe says why in an `#[expect]`.
#![cfg_attr(
    not(test),
    deny(
        clippy::indexing_slicing,
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::arithmetic_side_effects
    )
)]

use triple_buffer::{Input, Output, triple_buffer};

use crate::table::Table;

/// The UI thread's end.
#[derive(Debug)]
pub(crate) struct Publisher {
    input: Input<Table>,
}

/// The audio thread's end.
#[derive(Debug)]
pub(crate) struct Reader {
    output: Output<Table>,
}

pub(crate) fn snapshot() -> (Publisher, Reader) {
    let (input, output) = triple_buffer(&Table::default());
    (Publisher { input }, Reader { output })
}

impl Publisher {
    /// Makes `table` the one the next read takes. Drops a stale table, here
    /// on the UI thread.
    pub(crate) fn publish(&mut self, table: Table) {
        self.input.write(table);
    }
}

impl Reader {
    /// The newest published table. Wait-free; never allocates or frees.
    pub(crate) fn read(&mut self) -> &Table {
        self.output.read()
    }
}
