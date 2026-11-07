//! Learn's audio entry.

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

use seam_params::{AudioParams, audio_thread};
use seam_ump::EventList;

use crate::capture::Stager;
use crate::parse::Parser;
use crate::snapshot::Reader;
use crate::table::Route;

/// The audio thread's side of learn: [`process`](Self::process) runs once
/// per block.
///
/// Drop it off the audio thread: it may hold the last reference to the
/// published tables and to the parameters.
///
/// # Panics
///
/// In a build with `debug_assertions`, dropping it panics while the thread
/// holds an [`audio_thread`] mark.
///
/// # Examples
///
/// ```
/// use seam_learn::{Control, Mapping, Source};
#[doc = include_str!("../doc/setup.rs")]
/// # use seam_learn::Config;
/// # use seam_learn::Learn;
/// # let (mut learn, mut audio) = Learn::new(&set, Config::default());
/// learn.map(gain, Mapping::new(Source::any(Control::Cc(7))))?;
///
/// // The processor calls this once per block, before it reads its
/// // parameters: here a block that moves controller 7 to 127.
/// audio.process(&block(&[(7, 127)]));
/// assert_eq!(set.value(gain), 1.0);
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
#[derive(Debug)]
pub struct LearnAudio {
    params: AudioParams,
    reader: Reader,
    parser: Parser,
    stager: Stager,
}

impl Drop for LearnAudio {
    fn drop(&mut self) {
        audio_thread::assert_not_here("dropping LearnAudio");
    }
}

impl LearnAudio {
    pub(crate) fn new(params: AudioParams, reader: Reader, stager: Stager) -> Self {
        LearnAudio {
            params,
            reader,
            parser: Parser::new(),
            stager,
        }
    }

    /// Runs learn over one block's MIDI, in order, so a target that several
    /// messages reach ends the block on the last one.
    ///
    /// It takes the newest table the UI side published, without waiting,
    /// locking, allocating or freeing. Each mapped parameter is written in
    /// the block its message arrived in, as a learned write: no gesture, no
    /// undo step, nothing told to the host. A mapped UI-tier target gets the
    /// newest value staged for [`Learn::poll`](crate::Learn::poll), and while
    /// a target is armed the newest legal message is staged for the poll to
    /// learn. The list is read and never changed, so the processor gets every
    /// packet as the host sent it. The [`LearnAudio`] example shows a block.
    ///
    /// Thread: audio.
    #[cfg_attr(seam_rtsan, sanitize(realtime = "nonblocking"))]
    pub fn process(&mut self, events: &EventList) {
        let table = self.reader.read();
        self.stager.block(|offers| {
            for event in events.as_slice() {
                let Some(message) = self.parser.parse(event) else {
                    continue;
                };
                offers.offer(&message);
                table.dispatch(&message, |route, value| match route {
                    Route::Param(target) => self.params.learned_write(*target, value),
                    Route::Staged(staging) => staging.stage(value),
                });
            }
        });
    }

    /// Moves to the newest table the UI side published, as
    /// [`process`](Self::process) does at each block, and returns how many
    /// slots it holds: one per mapped target.
    ///
    /// A hook for tests, which have no other way to see the table the audio
    /// side holds.
    #[doc(hidden)]
    pub fn refresh_slots(&mut self) -> usize {
        self.reader.read().len()
    }
}
