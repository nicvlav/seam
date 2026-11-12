use std::num::NonZeroUsize;
use std::sync::Arc;

use seam_learn::{Config, Control, Learn, Learned, Mapping, Source, Takes};
use seam_params::{Kind, ParamDecl, ParamId, ParamSet};
use seam_ump::{ChannelVoice, Voice};

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

/// Records, for each piece, the gain it read and the controllers it was
/// handed.
#[derive(Debug)]
struct Reads {
    set: Arc<ParamSet>,
    gain: ParamId,
    pieces: Vec<(f32, usize)>,
}

impl Processor for Reads {
    fn process(&mut self, block: Block<'_, '_>) -> Status {
        let events = block.events.as_slice().len();
        self.pieces.push((self.set.value(self.gain), events));
        Status::Continue
    }
}

/// A MIDI 1.0 control change at `offset`.
fn control_change(offset: u32, controller: u8, value: u8) -> Event {
    let voice = Voice::Midi1ControlChange { controller, value };
    ChannelVoice {
        group: 0,
        channel: 0,
        voice,
    }
    .event(offset)
}

/// A gain from 0 to 1 that starts at 0, learn over it, and block state whose
/// processor reads the gain, with learn's audio side in it, in pieces of up
/// to 64 frames.
fn learning() -> (Learn, Audio<Reads>, ParamId) {
    let decl = ParamDecl::new("gain", "Gain", Kind::continuous(0.0, 1.0, 0.0, None), 0.0);
    let set = ParamSet::from_decls([decl]).expect("valid");
    let gain = set.id("gain").expect("declared");
    let (learn, learn_audio) = Learn::new(&set, Config::default());
    let reads = Reads {
        set,
        gain,
        pieces: Vec::new(),
    };
    let max_frames = NonZeroUsize::new(64).expect("not 0");
    let mut audio = Audio::around(reads, Layout::new(0, 0), max_frames, 4, 0);
    audio.learn = Some(learn_audio);
    (learn, audio, gain)
}

#[test]
fn a_learned_controller_is_written_before_the_processor_reads_in_its_own_block() {
    let (mut learn, mut audio, gain) = learning();
    learn
        .map(gain, Mapping::new(Source::any(Control::Cc(7))))
        .expect("mapped");
    audio.events.push(control_change(10, 7, 127));
    // SAFETY: a layout of no channels, so no buffer is read or written.
    unsafe { pieces(&mut audio, 32, &[], &[]) };

    assert_eq!(
        audio.processor.pieces,
        [(1.0, 1)],
        "the controller's value, and the controller itself"
    );
}

#[test]
fn a_controller_in_a_later_piece_is_read_by_every_piece_of_its_block() {
    let (mut learn, mut audio, gain) = learning();
    learn
        .map(gain, Mapping::new(Source::any(Control::Cc(7))))
        .expect("mapped");
    audio.events.push(control_change(100, 7, 127));
    // SAFETY: as above.
    unsafe { pieces(&mut audio, 150, &[], &[]) };

    assert_eq!(
        audio.processor.pieces,
        [(1.0, 0), (1.0, 1), (1.0, 0)],
        "learn ran once over the whole block before the first piece, as the \
         host's automation lands, and the controller still reaches its piece"
    );
}

#[test]
fn a_controller_a_block_brings_while_a_target_is_armed_is_learned_at_the_next_poll() {
    let (mut learn, mut audio, gain) = learning();
    learn.arm(gain, Takes::CONTROLLER).expect("a target");
    audio.events.push(control_change(0, 74, 64));
    // SAFETY: as above.
    unsafe { pieces(&mut audio, 16, &[], &[]) };

    assert!(
        matches!(learn.poll(), Some(Learned::Mapped { .. })),
        "the block staged it, the poll commits it"
    );
    assert_eq!(
        learn.mapping(gain).map(|mapping| mapping.source.control),
        Some(Control::Cc(74))
    );
}
