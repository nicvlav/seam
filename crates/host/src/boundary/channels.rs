//! The channels a block hands the processor, in place, filled from the
//! host's buffers.

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

use std::num::NonZeroUsize;

#[cfg(test)]
mod tests;

/// The channels a block hands the processor, in place, filled from the
/// host's buffers.
///
/// Allocated at activation for the main bus's layout and the most frames a
/// block holds; filling it never allocates. Channel `c` is the host's
/// output buffer `c` when the host gave one that overlaps no earlier
/// channel's, and a scratch buffer otherwise. The host's input is copied in
/// unless it is already there (a host processing in place), and a channel
/// with no input starts zeroed.
///
/// Two host buffers that overlap without being one buffer break
/// [`run`](Self::run)'s contract. Even then no two channels share a float,
/// and within a piece an input that overlaps another channel is read before
/// anything writes. Across the pieces of a long block it may not be: a later
/// piece can read what an earlier one wrote, wrong audio but no unsound
/// access.
#[derive(Debug)]
pub(crate) struct Channels {
    inputs: usize,
    outputs: usize,
    max_frames: NonZeroUsize,
    /// One per channel of the block, for those the host gave no buffer.
    scratch: Box<[Box<[f32]>]>,
    /// The input, when a host's input buffer is another channel's output.
    staging: Box<[Box<[f32]>]>,
    /// Where each channel of the block lives this call.
    places: Vec<*mut f32>,
    /// Kept empty between calls; only its capacity is reused.
    slots: Vec<&'static mut [f32]>,
}

// SAFETY: the raw pointers in `places` are written and read only inside
// `run`, which takes `&mut self`; between calls they are stale and unread,
// and `slots` is empty. Everything else is owned buffers.
unsafe impl Send for Channels {}

/// The host's buffer lists for one call, which starts `offset` floats into
/// each buffer.
struct Host<'a> {
    inputs: &'a [*mut f32],
    outputs: &'a [*mut f32],
    offset: usize,
}

impl Host<'_> {
    /// Input `index` at the call's first frame, if the host gave one.
    fn input(&self, index: usize) -> Option<*mut f32> {
        at(self.inputs, index, self.offset)
    }

    /// Output `index` at the call's first frame, if the host gave one.
    fn output(&self, index: usize) -> Option<*mut f32> {
        at(self.outputs, index, self.offset)
    }
}

/// Buffer `index` of a host's list, `offset` floats on; `None` when the
/// host listed none there or a null one. Only the address is made: whether
/// it may be read is the caller's to know.
fn at(buffers: &[*mut f32], index: usize, offset: usize) -> Option<*mut f32> {
    buffers
        .get(index)
        .copied()
        .filter(|buffer| !buffer.is_null())
        .map(|buffer| buffer.wrapping_add(offset))
}

impl Channels {
    /// Room for a block of `max(inputs, outputs)` channels of up to
    /// `max_frames` frames.
    pub(crate) fn new(inputs: usize, outputs: usize, max_frames: NonZeroUsize) -> Self {
        let width = inputs.max(outputs);
        let buffers = |count: usize| {
            (0..count)
                .map(|_| vec![0.0; max_frames.get()].into_boxed_slice())
                .collect::<Box<[_]>>()
        };
        Channels {
            inputs,
            outputs,
            max_frames,
            scratch: buffers(width),
            staging: buffers(inputs),
            places: Vec::with_capacity(width),
            slots: Vec::with_capacity(width),
        }
    }

    /// The most frames one call takes.
    pub(crate) fn max_frames(&self) -> NonZeroUsize {
        self.max_frames
    }

    /// Fills the block's channels for `frames` frames starting at `offset`
    /// in the host's buffers, and hands them to `process`. Afterwards, any
    /// host output buffer past the plugin's outputs is zeroed.
    ///
    /// # Safety
    ///
    /// `frames` is at most [`max_frames`](Self::max_frames). Each non-null
    /// pointer in `inputs` is valid for reads, and in `outputs` for reads
    /// and writes, of `offset + frames` floats, for the whole call. Nothing
    /// else reads or writes them meanwhile. Any two of the buffers, in
    /// either list, are one buffer or share no float: an output may be the
    /// same as an input, as a host processing in place hands it.
    pub(crate) unsafe fn run<R>(
        &mut self,
        inputs: &[*mut f32],
        outputs: &[*mut f32],
        offset: usize,
        frames: usize,
        process: impl FnOnce(&mut [&mut [f32]]) -> R,
    ) -> R {
        debug_assert!(frames <= self.max_frames.get());
        let frames = frames.min(self.max_frames.get());
        let host = Host {
            inputs,
            outputs,
            offset,
        };
        self.place(&host, frames);
        // SAFETY: this function's contract, `frames` at most the scratch's,
        // and every place set for this call just now.
        unsafe { self.fill(&host, frames) };
        // SAFETY: each place holds `frames` floats: a host output by this
        // function's contract, or scratch by its allocation. `place` gave no
        // two channels a float in common, and nothing else touches them
        // until `process` returns.
        let result = unsafe { self.lend(frames, process) };
        // SAFETY: this function's contract; the block's slices are gone.
        unsafe { self.clear_extras(&host, frames) };
        result
    }

    /// Sets where each channel lives this call: the host's output when its
    /// `frames` floats overlap no earlier channel's, scratch otherwise. So
    /// no two channels share a float, whatever buffers the host hands.
    fn place(&mut self, host: &Host<'_>, frames: usize) {
        self.places.clear();
        for (index, scratch) in self.scratch.iter_mut().enumerate() {
            let own = host
                .output(index)
                .filter(|_| index < self.outputs)
                .filter(|&own| !self.places.iter().any(|&place| overlap(place, own, frames)));
            self.places.push(own.unwrap_or(scratch.as_mut_ptr()));
        }
    }

    /// Copies the host's input into each channel, and zeroes a channel with
    /// none.
    ///
    /// When a host input overlaps another channel's place, every input is
    /// staged first, so no copy or zeroing overwrites an input not yet read.
    ///
    /// # Safety
    ///
    /// As [`run`](Self::run), with `frames` at most the scratch's and every
    /// place set for this call.
    unsafe fn fill(&mut self, host: &Host<'_>, frames: usize) {
        let crossed = self.crossed(host, frames);
        if crossed {
            // SAFETY: this function's contract.
            unsafe { self.stage(host, frames) };
        }
        for (index, &place) in self.places.iter().enumerate() {
            match host.input(index).filter(|_| index < self.inputs) {
                // Every input comes from staging, one already in place too.
                // That copy rewrites what is there, and keeps the mode one
                // rule: no host input is read once anything is written.
                Some(_) if crossed => {
                    // Staging has a buffer for each of the plugin's inputs.
                    if let Some(staged) = self.staging.get(index) {
                        // SAFETY: staging holds the `frames` floats `stage`
                        // copied in; the place holds `frames` (this
                        // function's contract, or scratch's allocation);
                        // staging is an allocation of its own.
                        unsafe { std::ptr::copy_nonoverlapping(staged.as_ptr(), place, frames) };
                    }
                }
                Some(source) if source != place => {
                    // SAFETY: both hold `frames` floats: the host's by the
                    // contract, a scratch place by its allocation. `copy`
                    // allows an input that overlaps its own channel's place,
                    // which a host breaking the contract may hand.
                    unsafe { std::ptr::copy(source, place, frames) };
                }
                // In place: the input is already where the channel lives.
                Some(_) => {}
                None => {
                    // SAFETY: the place holds `frames` floats, as above.
                    unsafe { std::ptr::write_bytes(place, 0, frames) };
                }
            }
        }
    }

    /// Whether a host input's `frames` floats overlap another channel's
    /// place, as when a host hands its left input as the right output.
    fn crossed(&self, host: &Host<'_>, frames: usize) -> bool {
        (0..self.inputs).any(|index| {
            host.input(index).is_some_and(|source| {
                self.places
                    .iter()
                    .enumerate()
                    .any(|(other, &place)| other != index && overlap(place, source, frames))
            })
        })
    }

    /// Copies every host input into staging.
    ///
    /// # Safety
    ///
    /// As [`fill`](Self::fill).
    unsafe fn stage(&mut self, host: &Host<'_>, frames: usize) {
        for (index, staging) in self.staging.iter_mut().enumerate() {
            if let Some(source) = host.input(index) {
                // SAFETY: `source` holds `frames` floats (the contract);
                // `staging` was allocated with `max_frames`, at least
                // `frames`; they are separate allocations.
                unsafe { std::ptr::copy_nonoverlapping(source, staging.as_mut_ptr(), frames) };
            }
        }
    }

    /// Hands `process` the block: a slice over each channel's place.
    ///
    /// # Safety
    ///
    /// Each place holds `frames` floats, no two share a float, and nothing
    /// else touches them until `process` returns.
    unsafe fn lend<R>(&mut self, frames: usize, process: impl FnOnce(&mut [&mut [f32]]) -> R) -> R {
        let mut slots = relifetime(std::mem::take(&mut self.slots));
        for &place in &self.places {
            // SAFETY: this function's contract. The slices end with `slots`,
            // emptied below once `process` returns.
            slots.push(unsafe { std::slice::from_raw_parts_mut(place, frames) });
        }
        let result = process(&mut slots);
        self.slots = relifetime(slots);
        result
    }

    /// Zeroes each host output buffer past the plugin's outputs that
    /// overlaps no channel's place, so the block's output stands.
    ///
    /// # Safety
    ///
    /// As [`run`](Self::run), with no slice over a place left.
    unsafe fn clear_extras(&self, host: &Host<'_>, frames: usize) {
        for index in self.outputs..host.outputs.len() {
            if let Some(extra) = host.output(index).filter(|&extra| {
                !self
                    .places
                    .iter()
                    .any(|&place| overlap(place, extra, frames))
            }) {
                // SAFETY: a host output buffer, valid for writing `frames`
                // floats from the call's offset (the contract), and sharing
                // none with the block's channels.
                unsafe { std::ptr::write_bytes(extra, 0, frames) };
            }
        }
    }
}

/// Whether `frames` floats from `a` and `frames` from `b` share one, or the
/// two start at one address. Only the addresses are compared, so two
/// pointers into different allocations never overlap.
fn overlap(a: *mut f32, b: *mut f32, frames: usize) -> bool {
    let span = frames.saturating_mul(size_of::<f32>());
    let (a, b) = (a.addr(), b.addr());
    a == b || (a < b.saturating_add(span) && b < a.saturating_add(span))
}

/// Empties a list of slices and hands it back as a list for another
/// lifetime, so its capacity is reused across calls.
fn relifetime<'a, 'b>(mut slots: Vec<&'a mut [f32]>) -> Vec<&'b mut [f32]> {
    slots.clear();
    // SAFETY: the two types differ only in a lifetime, which has no effect
    // on layout, and the list is empty, so it holds no slice that could
    // outlive its own.
    unsafe { std::mem::transmute::<Vec<&'a mut [f32]>, Vec<&'b mut [f32]>>(slots) }
}
