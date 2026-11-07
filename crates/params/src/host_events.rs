//! What edits tell the host, from the UI thread to the format exporter's
//! drain.
//!
//! Each parameter has one atomic word of flags and one value cell, so the
//! queue holds at most one entry per parameter, never fills, and never drops
//! a begin or an end. Neither side locks or allocates, so CLAP can drain it on
//! the audio thread and VST3 on the UI thread.

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

use std::sync::Arc;

#[cfg(loom)]
use loom::sync::atomic::{AtomicBool, AtomicU32, Ordering};
#[cfg(not(loom))]
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

use crate::set::cell;
use crate::{HostEvent, ParamId, ParamSet, audio_thread};

/// A gesture the host has been told of is open now.
const OPEN: u32 = 1;
/// A gesture opened since the last drain.
const OPENED: u32 = 1 << 1;
/// A value was written since the last drain.
const VALUE: u32 = 1 << 2;

/// The producer side, inside the parameter set.
#[derive(Debug)]
pub(crate) struct HostQueue {
    flags: Box<[AtomicU32]>,
    values: Box<[AtomicU32]>,
    /// Some flag was set since the last drain, so an idle drain is one swap
    /// that finds it clear.
    dirty: AtomicBool,
    rescan: AtomicBool,
    /// Whether a [`HostEventDrain`] is alive: there is one per set, so a
    /// second take gives `None` until it drops.
    drain_alive: AtomicBool,
}

impl HostQueue {
    pub(crate) fn new(len: usize) -> Self {
        Self {
            flags: (0..len).map(|_| AtomicU32::new(0)).collect(),
            values: (0..len).map(|_| AtomicU32::new(0)).collect(),
            dirty: AtomicBool::new(false),
            rescan: AtomicBool::new(false),
            drain_alive: AtomicBool::new(false),
        }
    }

    /// The host hears a begin before the gesture's first value.
    pub(crate) fn begin(&self, id: ParamId) {
        cell(&self.flags, id).fetch_or(OPEN | OPENED, Ordering::Release);
        self.mark_dirty();
    }

    /// `value` is the snapped value, in real units.
    pub(crate) fn value(&self, id: ParamId, value: f32) {
        cell(&self.values, id).store(value.to_bits(), Ordering::Relaxed);
        cell(&self.flags, id).fetch_or(VALUE, Ordering::Release);
        self.mark_dirty();
    }

    /// The host hears an end after the gesture's last value.
    pub(crate) fn end(&self, id: ParamId) {
        cell(&self.flags, id).fetch_and(!OPEN, Ordering::Release);
        self.mark_dirty();
    }

    /// A read-modify-write, not a store, as the drain's swap is. Loom 0.7.2
    /// can lose a plain store racing an RMW on the same atomic
    /// and report an outcome the memory model forbids.
    ///
    /// Release here, with acquire on the drain's swap, makes the flags set
    /// before the mark happen before the drain's reads of them. A drain that
    /// clears the mark therefore sees every flag behind it. Without them its
    /// read of a flag could come first in that flag's order, and the event
    /// would wait until a later write marks the queue again. Loom cannot see
    /// this: its read-modify-writes always read the newest value.
    fn mark_dirty(&self) {
        self.dirty.swap(true, Ordering::Release);
    }

    /// A sync asks the host to re-read every value. Called after the
    /// sync's store, so a reader that takes the request sees the value.
    pub(crate) fn request_rescan(&self) {
        self.rescan.store(true, Ordering::Release);
    }

    pub(crate) fn take_rescan(&self) -> bool {
        self.rescan.swap(false, Ordering::AcqRel)
    }
}

/// The format exporter's drain of what edits tell the host. One per
/// [`ParamSet`].
///
/// Each parameter reaches the host as balanced, non-overlapping begin and end
/// pairs with values only inside them, whatever happened between two drains:
/// values coalesce to the latest, and a gesture that closed and reopened
/// between two drains stays one bracket. Before dropping the drain,
/// [`finish`](Self::finish) it, so the host hears an end for every bracket it
/// heard begin.
///
/// # Panics
///
/// In a build with `debug_assertions`, dropping it panics while the thread
/// holds an [`audio_thread`] mark, or while the host
/// has heard a begin with no end: [`finish`](Self::finish) it first.
///
/// # Examples
///
/// ```
/// use seam_params::{HostEvent, Kind, ParamDecl, ParamSet};
///
/// let set = ParamSet::from_decls([
///     ParamDecl::new("gain", "Gain", Kind::continuous(-60.0, 12.0, 0.1, None), 0.0),
/// ])?;
/// let gain = set.id("gain").expect("declared above");
/// let ui = set.take_ui().expect("the first UI handle");
///
/// let mut drain = set.take_host_event_drain().expect("the first drain");
/// assert!(set.take_host_event_drain().is_none()); // one per set
///
/// let drag = ui.begin(gain);
/// drag.write(-6.0);
/// let mut heard = Vec::new();
/// drain.drain(|event| heard.push(event));
/// assert_eq!(heard.len(), 2); // a begin and the value; the drag has not ended
///
/// // The host stops listening mid-drag: it still hears the end.
/// drain.finish(|event| heard.push(event));
/// assert_eq!(heard.last(), Some(&HostEvent::End(gain)));
/// drop(drag);
/// # Ok::<(), seam_params::DeclError>(())
/// ```
#[derive(Debug)]
pub struct HostEventDrain {
    set: Arc<ParamSet>,
    /// Which parameters the host has heard a begin for and no end.
    host_open: Box<[bool]>,
}

impl HostEventDrain {
    pub(crate) fn new(set: &Arc<ParamSet>) -> Option<Self> {
        let queue = set.host();
        if queue.drain_alive.swap(true, Ordering::AcqRel) {
            return None;
        }
        // What happened before this drain existed is stale for its host.
        // A gesture open now still reaches it as a begin at the first drain,
        // which scans every parameter even if an earlier drain took the
        // dirty mark.
        for flags in &queue.flags {
            flags.fetch_and(OPEN, Ordering::AcqRel);
        }
        queue.mark_dirty();
        Some(Self {
            set: Arc::clone(set),
            host_open: vec![false; set.len()].into_boxed_slice(),
        })
    }

    /// Hands every event since the last drain to `emit`, per parameter in
    /// order: a begin, the latest value, an end. Wait-free and
    /// allocation-free, so it may run on the audio thread.
    ///
    /// If `emit` panics, the events it has not yet returned from stay
    /// queued: the next drain hands them out, so the host still hears a
    /// balanced bracket. [`HostEventDrain`] shows it called.
    #[cfg_attr(seam_rtsan, sanitize(realtime = "nonblocking"))]
    pub fn drain(&mut self, mut emit: impl FnMut(HostEvent)) {
        let queue = self.set.host();
        if !queue.dirty.swap(false, Ordering::Acquire) {
            return;
        }
        let mut unwind = Requeue {
            queue,
            index: 0,
            unemitted: 0,
            armed: true,
        };
        let cells = queue.flags.iter().zip(&queue.values);
        for (index, (host_open, (flag, stored))) in self.host_open.iter_mut().zip(cells).enumerate()
        {
            let flags = flag.fetch_and(OPEN, Ordering::AcqRel);
            let (open, opened, value) = (
                (flags & OPEN) != 0,
                (flags & OPENED) != 0,
                (flags & VALUE) != 0,
            );
            unwind.index = index;
            unwind.unemitted = flags & (OPENED | VALUE);
            let id = ParamId::from_index(index);
            if !*host_open && (opened || open || value) {
                emit(HostEvent::Begin(id));
                *host_open = true;
                unwind.unemitted &= !OPENED;
            }
            if value {
                let real = f32::from_bits(stored.load(Ordering::Relaxed));
                emit(HostEvent::Value {
                    id,
                    normalized: self.set.decl(id).kind.normalize(real),
                });
                unwind.unemitted &= !VALUE;
            }
            if *host_open && !open {
                emit(HostEvent::End(id));
                *host_open = false;
            }
        }
        unwind.armed = false;
    }
}

/// Puts back what a drain took but has not finished handing out, if the
/// drain unwinds. The parameter it was on keeps its unemitted flags, and the
/// queue is marked dirty so the parameters after it are scanned again.
struct Requeue<'a> {
    queue: &'a HostQueue,
    index: usize,
    unemitted: u32,
    armed: bool,
}

impl Drop for Requeue<'_> {
    fn drop(&mut self) {
        if self.armed {
            if let Some(flag) = self.queue.flags.get(self.index)
                && self.unemitted != 0
            {
                flag.fetch_or(self.unemitted, Ordering::Release);
            }
            self.queue.mark_dirty();
        }
    }
}

impl HostEventDrain {
    /// Drains, then ends every bracket the host still has open.
    ///
    /// The exporter calls it when the host stops listening (a CLAP deactivate,
    /// a VST3 disconnect), so the host never holds a begin with no end.
    /// [`HostEventDrain`] shows it after a drag the host stopped hearing.
    pub fn finish(&mut self, mut emit: impl FnMut(HostEvent)) {
        self.drain(&mut emit);
        for (index, host_open) in self.host_open.iter_mut().enumerate() {
            if *host_open {
                emit(HostEvent::End(ParamId::from_index(index)));
                *host_open = false;
            }
        }
    }
}

impl Drop for HostEventDrain {
    fn drop(&mut self) {
        audio_thread::assert_not_here("dropping a HostEventDrain");
        debug_assert!(
            std::thread::panicking() || self.host_open.iter().all(|open| !open),
            "a host event drain dropped with a bracket open; call finish first"
        );
        self.set.host().drain_alive.store(false, Ordering::Release);
    }
}

#[cfg(all(test, loom))]
mod loom_models {
    use std::sync::Arc;

    use crate::{HostEvent, Kind, ParamDecl, ParamId, ParamSetBuilder};

    /// The parameters' host event queue: a gesture on the UI thread races
    /// the exporter's drain. Whatever the interleaving, the host hears one
    /// balanced bracket with values only inside it, ending on the gesture's
    /// last value.
    #[test]
    fn loom_a_drain_racing_a_gesture_hears_one_balanced_bracket() {
        loom::model(|| {
            let mut builder = ParamSetBuilder::new();
            builder
                .add(ParamDecl::new("p", "p", Kind::Bool, 0.0))
                .expect("valid");
            let set = builder.build();
            let mut drain = set.take_host_event_drain().expect("the first drain");
            let producer = Arc::clone(&set);
            let thread = loom::thread::spawn(move || {
                let id = ParamId::from_index(0);
                producer.host().begin(id);
                producer.host().value(id, 1.0);
                producer.host().end(id);
            });
            let mut events = Vec::new();
            drain.drain(|event| events.push(event));
            thread.join().expect("the UI thread");
            drain.drain(|event| events.push(event));
            // Both drains have heard everything, so finishing closes nothing.
            drain.finish(|event| panic!("finish closed a bracket the drains left open: {event:?}"));

            let mut open = false;
            for event in &events {
                match event {
                    HostEvent::Begin(_) => {
                        assert!(!open, "a begin inside a bracket: {events:?}");
                        open = true;
                    }
                    HostEvent::Value { .. } => assert!(open, "a value outside: {events:?}"),
                    HostEvent::End(_) => {
                        assert!(open, "an end with no begin: {events:?}");
                        open = false;
                    }
                }
            }
            let last = events.iter().rev().find_map(|event| match event {
                HostEvent::Value { normalized, .. } => Some(*normalized),
                _ => None,
            });
            assert_eq!(last, Some(1.0), "{events:?}");
            assert!(!open);
        });
    }
}
