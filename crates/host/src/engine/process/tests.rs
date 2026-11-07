use std::num::NonZeroUsize;

use super::{Audio, pieces};
use crate::{Block, Event, Layout, Processor, Status};

/// What one piece handed the processor: its frames, each event's offset and
/// tag, and the drops its list reported.
#[derive(Debug, PartialEq)]
struct Piece {
    frames: usize,
    events: Vec<(u32, u32)>,
    dropped: u32,
}

/// Records each piece, and sends every event it gets back at the same
/// frame of its piece.
#[derive(Debug, Default)]
struct Echo {
    pieces: Vec<Piece>,
}

impl Processor for Echo {
    fn process(&mut self, block: Block<'_, '_>) -> Status {
        let events = block.events.as_slice().iter().map(tagged).collect();
        self.pieces.push(Piece {
            frames: block.frames,
            events,
            dropped: block.events.dropped(),
        });
        for &event in block.events.as_slice() {
            block.out_events.push(event);
        }
        Status::Continue
    }
}

/// A note-on at `offset` whose velocity is `tag`, so a test can follow it.
fn note(offset: u32, tag: u32) -> Event {
    Event {
        offset,
        words: [0x2090_3C00 | tag, 0, 0, 0],
    }
}

fn tagged(event: &Event) -> (u32, u32) {
    (event.offset, event.words[0] & 0x7F)
}

/// Room for 6 events in and 5 out, in pieces of up to 64 frames.
fn audio() -> Audio<Echo> {
    let max_frames = NonZeroUsize::new(64).expect("not 0");
    Audio::around(Echo::default(), Layout::new(0, 0), max_frames, 6, 5)
}

#[test]
fn each_piece_gets_its_own_events_and_what_it_sends_lands_at_the_block_s_frames() {
    let mut audio = audio();
    // In delivery order. One is past the block's end, and the last has no
    // room in the block's list.
    for (offset, tag) in [
        (10, 1),
        (63, 2),
        (64, 3),
        (100, 4),
        (149, 5),
        (400, 6),
        (20, 7),
    ] {
        audio.events.push(note(offset, tag));
    }
    // SAFETY: a layout of no channels, so no buffer is read or written.
    let status = unsafe { pieces(&mut audio, 150, &[], &[]) };
    assert_eq!(status, Status::Continue);

    let piece = |frames, events: &[(u32, u32)], dropped| Piece {
        frames,
        events: events.to_vec(),
        dropped,
    };
    assert_eq!(
        audio.processor.pieces,
        [
            piece(64, &[(10, 1), (63, 2)], 1),
            piece(64, &[(0, 3), (36, 4)], 0),
            // The event past the end lands at the block's last frame.
            piece(22, &[(21, 5), (21, 6)], 0),
        ],
        "each piece its own events, from its own start; the first reports \
         the block's drops"
    );
    let sent: Vec<_> = audio.out_events.as_slice().iter().map(tagged).collect();
    assert_eq!(sent, [(10, 1), (63, 2), (64, 3), (100, 4), (149, 5)]);
    assert_eq!(audio.out_events.dropped(), 1, "the sixth had no room");
}

#[test]
fn a_block_of_one_piece_hands_its_events_on_and_the_next_starts_clean() {
    let mut audio = audio();
    audio.events.push(note(5, 1));
    audio.events.push(note(80, 2));
    // SAFETY: as above.
    unsafe { pieces(&mut audio, 32, &[], &[]) };
    audio.events.clear();
    // SAFETY: as above.
    unsafe { pieces(&mut audio, 16, &[], &[]) };

    assert_eq!(
        audio.processor.pieces,
        [
            Piece {
                frames: 32,
                events: vec![(5, 1), (31, 2)],
                dropped: 0,
            },
            Piece {
                frames: 16,
                events: Vec::new(),
                dropped: 0,
            },
        ]
    );
    assert!(
        audio.out_events.as_slice().is_empty(),
        "the last block sent none"
    );
}
