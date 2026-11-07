//! The block: what the audio thread works with, and one call of it.

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

#[cfg(loom)]
use loom::sync::atomic::AtomicBool;
use std::num::NonZeroUsize;
use std::panic::{self, AssertUnwindSafe};
#[cfg(not(loom))]
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;

use seam_params::audio_thread;

use super::{Engine, HostLink, Points, Setup};
use crate::boundary::{AudioSlot, Channels, FlushDenormals};
use crate::faults::Fault;
use crate::{Block, Event, EventList, Layout, Plugin, ProcessSetup, Processor, Status};

/// What the audio thread works with while the plugin is active.
#[derive(Debug)]
pub(super) struct Audio<R> {
    pub(super) processor: R,
    channels: Channels,
    /// The block's MIDI in, at frames from the block's start.
    events: EventList,
    /// One piece's share of `events`, at frames from the piece's start. It
    /// has the room `events` has, so a piece's share never drops.
    piece_events: EventList,
    /// The block's MIDI out. Every piece pushes to it, and what a piece
    /// pushed moves to the block's frames once the piece returns.
    out_events: EventList,
    /// Set when a block panicked in this processor, which then plays
    /// silence. The flag lives with the processor, so the one an activation
    /// builds starts clean.
    faulted: bool,
}

impl<R> Audio<R> {
    /// Asks `plugin` for a processor for `setup` and `layout`, and allocates
    /// everything a block will touch. `events` gives the capacity of each
    /// direction's MIDI list from whether the plugin declares MIDI there.
    pub(super) fn new<P: Plugin<Processor = R>>(
        plugin: &mut P,
        setup: Setup,
        layout: Layout,
        events: impl Fn(bool) -> usize,
    ) -> Self {
        let max_events = events(P::INFO.midi_input);
        let max_out_events = events(P::INFO.midi_output);
        let processor = plugin.activate(&ProcessSetup {
            sample_rate: setup.sample_rate,
            max_frames: setup.max_frames.get(),
            layout,
            max_events,
            max_out_events,
        });
        Audio::around(
            processor,
            layout,
            setup.max_frames,
            max_events,
            max_out_events,
        )
    }

    fn around(
        processor: R,
        layout: Layout,
        max_frames: NonZeroUsize,
        max_events: usize,
        max_out_events: usize,
    ) -> Self {
        Audio {
            processor,
            channels: Channels::new(layout.inputs, layout.outputs, max_frames),
            events: EventList::with_capacity(max_events),
            piece_events: EventList::with_capacity(max_events),
            out_events: EventList::with_capacity(max_out_events),
            faulted: false,
        }
    }
}

/// What a block found in the audio slot.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum Held<T> {
    /// The processor ran the block, which came to this.
    Ran(T),
    /// The processor panicked in the block; it is marked faulted now.
    Panicked,
    /// No processor, one that panicked before, or a slot an activation
    /// holds: nothing ran.
    Idle,
}

/// Runs `block` on the processor in `slot`, when no one else holds the slot,
/// there is a processor and it never panicked.
///
/// A panic in `block` is caught here, while the slot is still held, and
/// marks that processor faulted, so the mark stays with it. An activation,
/// which must wait for the slot to replace the processor, installs one that
/// starts clean, however the two threads interleave.
pub(super) fn run_held<R, T>(
    slot: &AudioSlot<Option<Audio<R>>>,
    block: impl FnOnce(&mut Audio<R>) -> T,
) -> Held<T> {
    let Some(mut held) = slot.try_hold() else {
        return Held::Idle;
    };
    let Some(audio) = held.as_mut() else {
        return Held::Idle;
    };
    if audio.faulted {
        return Held::Idle;
    }
    match panic::catch_unwind(AssertUnwindSafe(|| block(&mut *audio))) {
        Ok(result) => Held::Ran(result),
        Err(_) => {
            audio.faulted = true;
            Held::Panicked
        }
    }
}

/// A reset asked of the processor in the audio slot, which the next block
/// on it runs first.
pub(super) struct PendingReset(AtomicBool);

impl PendingReset {
    pub(super) fn new() -> Self {
        PendingReset(AtomicBool::new(false))
    }

    /// Asks the next block to reset its processor first. Any thread.
    pub(super) fn ask(&self) {
        self.0.store(true, Ordering::Release);
    }

    /// Whether a reset was asked, clearing the request: for a block that
    /// holds the slot.
    fn take(&self) -> bool {
        self.0.swap(false, Ordering::AcqRel)
    }
}

/// Puts `audio` in `slot` for an activation, returning the processor it
/// replaces. A reset asked of that one is dropped, since the new one has
/// nothing to clear.
///
/// The request is cleared while the slot is held. A block reaches the new
/// processor only by holding the slot after that, so a reset asked once a
/// block has run on it is kept for the next one.
pub(super) fn install<R>(
    slot: &AudioSlot<Option<Audio<R>>>,
    reset: &PendingReset,
    audio: Audio<R>,
) -> Option<Audio<R>> {
    let mut held = slot.hold();
    reset.0.store(false, Ordering::Release);
    held.replace(audio)
}

/// What a block came to, for the shell to answer its host with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Outcome {
    /// A block of no frames: the host's writes landed and nothing played.
    Empty,
    /// The block did not play: the shell writes silence to every output.
    Silent,
    /// The processor played it, answering this for its last piece.
    Played(Status),
}

impl<P: Plugin, H: HostLink> Engine<P, H> {
    /// One block of `frames` frames.
    ///
    /// First comes the host's automation for it, which `automation` hands over
    /// queue by queue and which lands before the block. Then the processor
    /// runs, in pieces when the block is longer than the activation promised.
    ///
    /// A panic in the processor is caught while it is still held (see
    /// [`run_held`]): the block is silent, and so is every block until the
    /// next activation's processor. A panic anywhere else in the block, the
    /// automation walk say, is caught here and silences that block alone.
    ///
    /// The calling thread carries the params crate's audio-thread mark
    /// throughout, so a debug build panics, caught as above, at any lock or
    /// audio-side drop the block reaches. Nothing here drops a processor:
    /// one that panicked stays in its slot until the main thread replaces
    /// or takes it.
    ///
    /// # Safety
    ///
    /// Each non-null pointer in `inputs` is valid for reading, and in
    /// `outputs` for reading and writing, `frames` floats until this call
    /// returns, and nothing else reads or writes them meanwhile. Any two of
    /// them are one buffer or share no float: an output may be the same
    /// buffer as an input, as a host processing in place hands it.
    #[cfg_attr(seam_rtsan, sanitize(realtime = "nonblocking"))]
    pub(crate) unsafe fn process(
        &self,
        frames: usize,
        inputs: &[*mut f32],
        outputs: &[*mut f32],
        automation: impl FnOnce(&mut Points<'_>),
    ) -> Outcome {
        let _audio = audio_thread::enter();
        // SAFETY: this function's contract, passed on whole.
        let block = || unsafe { self.play(frames, inputs, outputs, automation) };
        panic::catch_unwind(AssertUnwindSafe(block)).unwrap_or_else(|_| {
            self.faults.record(Fault::Panic);
            Outcome::Silent
        })
    }

    /// The block, with only the processor's panics caught.
    ///
    /// # Safety
    ///
    /// As [`process`](Self::process).
    #[cfg_attr(seam_rtsan, sanitize(realtime = "nonblocking"))]
    unsafe fn play(
        &self,
        frames: usize,
        inputs: &[*mut f32],
        outputs: &[*mut f32],
        automation: impl FnOnce(&mut Points<'_>),
    ) -> Outcome {
        {
            let mut room = self.automation.try_hold();
            let mut points = Points::new(room.as_deref_mut(), &self.host);
            automation(&mut points);
            points.apply();
        }
        // After the host's writes, as JUCE's wrapper reads them outside the
        // plugin's `processBlock` and its `ScopedNoDenormals`.
        let _flush = FlushDenormals::new();
        if frames == 0 {
            return Outcome::Empty;
        }
        let held = run_held(&self.audio, |audio| {
            if self.reset.take() {
                audio.processor.reset();
            }
            // An exporter's MIDI in is to fill the list here; none does yet.
            audio.events.clear();
            // SAFETY: this function's contract.
            unsafe { pieces(audio, frames, inputs, outputs) }
        });
        match held {
            Held::Ran(status) => Outcome::Played(status),
            Held::Panicked => {
                self.faults.record(Fault::AudioPanic);
                Outcome::Silent
            }
            Held::Idle => Outcome::Silent,
        }
    }
}

/// The processor over the block, in pieces of at most the activation's
/// frames, answering the last piece's status.
///
/// Each piece is handed the block's events that fall in it, at frames from
/// its own start. An event past the block's end plays at its last frame, in
/// the last piece: dropping it could lose a note-off and leave a note
/// hanging. The first piece also reports the block's drops. They were lost
/// before any piece ran, at frames no one knows, and the first piece is the
/// earliest a processor can act on them. Reported once, they add up over the
/// pieces to the block's count.
///
/// Every piece sends into the block's list, and what it sent moves to the
/// block's frames once it returns. A later piece finds the earlier pieces'
/// events already there.
///
/// # Safety
///
/// As [`Engine::process`].
unsafe fn pieces<R: Processor>(
    audio: &mut Audio<R>,
    frames: usize,
    inputs: &[*mut f32],
    outputs: &[*mut f32],
) -> Status {
    let Audio {
        processor,
        channels,
        events,
        piece_events,
        out_events,
        ..
    } = audio;
    out_events.clear();
    let mut status = Status::Continue;
    let mut offset = 0;
    while offset < frames {
        // At least a frame, since the activation's frames are never 0, so
        // the loop ends.
        let length = frames
            .saturating_sub(offset)
            .min(channels.max_frames().get());
        share(events, piece_events, frames, offset, length);
        let sent = out_events.as_slice().len();
        // SAFETY: each non-null channel holds `frames` floats (this
        // function's contract), so `offset + length` of them, nothing else
        // touches them during the call, and `length` is at most the
        // scratch's frames.
        status = unsafe {
            channels.run(inputs, outputs, offset, length, |block| {
                processor.process(Block::new(block, length, piece_events, out_events))
            })
        };
        let start = u32::try_from(offset).unwrap_or(u32::MAX);
        for event in out_events.as_mut_slice().iter_mut().skip(sent) {
            event.offset = event.offset.saturating_add(start);
        }
        offset = offset.saturating_add(length);
    }
    status
}

/// Fills `piece` with its share of a block of `frames` frames, for the
/// piece of `length` frames from `offset`, as [`pieces`] says.
fn share(block: &EventList, piece: &mut EventList, frames: usize, offset: usize, length: usize) {
    piece.clear();
    if offset == 0 {
        piece.add_dropped(block.dropped());
    }
    let last = frames.saturating_sub(1);
    let end = offset.saturating_add(length);
    for event in block.as_slice() {
        let at = usize::try_from(event.offset).map_or(last, |at| at.min(last));
        if (offset..end).contains(&at) {
            piece.push(Event {
                offset: u32::try_from(at.saturating_sub(offset)).unwrap_or(u32::MAX),
                ..*event
            });
        }
    }
}

#[cfg(test)]
mod tests;

#[cfg(all(test, loom))]
mod loom_models {
    use loom::sync::Arc;

    use std::num::NonZeroUsize;

    use super::{Audio, Held, PendingReset, install, run_held};
    use crate::Layout;
    use crate::boundary::AudioSlot;

    /// A processor that says which activation built it.
    #[derive(Debug)]
    struct Built(u32);

    fn audio(activation: u32) -> Option<Audio<Built>> {
        Some(Audio::around(
            Built(activation),
            Layout::new(0, 0),
            NonZeroUsize::MIN,
            0,
            0,
        ))
    }

    /// A block whose processor panics races an activation that replaces
    /// the processor. Whatever the interleaving, the fault marks the
    /// processor the block ran on and no other: the activation's own
    /// starts clean unless the block ran on it.
    #[test]
    fn loom_a_panicking_block_racing_an_activation_marks_only_its_own_processor() {
        loom::model(|| {
            let slot = Arc::new(AudioSlot::new(audio(1)));
            let audio_side = Arc::clone(&slot);
            let block = loom::thread::spawn(move || {
                let mut ran_on = None;
                let held = run_held(&audio_side, |audio: &mut Audio<Built>| {
                    ran_on = Some(audio.processor.0);
                    // Unwinds as a panic does, without the panic hook's
                    // message for every interleaving.
                    std::panic::resume_unwind(Box::new(()))
                });
                assert!(matches!(held, Held::Panicked | Held::Idle), "{held:?}");
                ran_on
            });
            let old = slot.hold().replace(audio(2).expect("built"));
            let ran_on = block.join().expect("the audio thread");

            let old = old.expect("the first processor");
            assert_eq!(old.faulted, ran_on == Some(1), "the first processor");
            let new = slot.try_hold().expect("free once both let go");
            let new = new.as_ref().expect("the activation's processor");
            assert_eq!(new.faulted, ran_on == Some(2), "the activation's processor");
        });
    }

    /// A block, then a reset asked once it is done, as a host's stop of
    /// processing asks one, race an activation that installs a processor
    /// and drops the resets asked of the one it replaces. Whatever the
    /// interleaving, once the block ran on the new processor the reset
    /// asked after it is kept for the next block.
    #[test]
    fn loom_a_reset_asked_after_a_block_on_a_new_processor_is_kept() {
        loom::model(|| {
            let slot = Arc::new(AudioSlot::new(audio(1)));
            let reset = Arc::new(PendingReset::new());
            let (audio_side, asked) = (Arc::clone(&slot), Arc::clone(&reset));
            let block = loom::thread::spawn(move || {
                // As a block takes the request before it plays.
                let held = run_held(&audio_side, |audio: &mut Audio<Built>| {
                    asked.take();
                    audio.processor.0
                });
                asked.ask();
                held
            });
            let old = install(&slot, &reset, audio(2).expect("built"));
            let held = block.join().expect("the audio thread");

            assert_eq!(old.map(|old| old.processor.0), Some(1));
            if held == Held::Ran(2) {
                assert!(reset.take(), "the reset asked after the new one ran");
            }
        });
    }
}
