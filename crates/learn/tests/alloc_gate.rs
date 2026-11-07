//! Learn's audio entry neither allocates nor frees, including the
//! first block after a publish, when it takes a new table.

seam_testkit::install_alloc_gate!();

use std::sync::Arc;

use seam_learn::{
    CcReservation, Config, Control, Learn, Learned, Mapping, NrpnReservation, Source, Takes, Target,
};
use seam_params::{Kind, ParamDecl, ParamSet, ParamSetBuilder, Range};
use seam_testkit::alloc::assert_no_allocations;
use seam_ump::{ChannelVoice, Event, EventList, Voice};

const TARGETS: usize = 256;

fn set() -> Arc<ParamSet> {
    let mut builder = ParamSetBuilder::new();
    for index in 0..TARGETS {
        let key: &'static str = Box::leak(format!("p{index}").into_boxed_str());
        builder
            .add(ParamDecl::new(
                key,
                key,
                Kind::Continuous(
                    Range::new(20.0, 20_000.0, 0.0, Some(1_000.0)).expect("a valid range"),
                ),
                1_000.0,
            ))
            .expect("valid");
    }
    builder.build()
}

fn source(index: usize) -> Source {
    let control = match index % 3 {
        0 => Control::Cc((index / 3 % 5) as u8 + 1),
        1 => Control::Nrpn(300 + (index / 3 % 2) as u16),
        _ => Control::Note(60 + (index / 3 % 2) as u8),
    };
    Source {
        control,
        group: index.is_multiple_of(2).then_some(0),
        channel: (index % 4 < 2).then_some(1),
    }
}

/// One block of everything learn reads, and some it does not.
fn events() -> EventList {
    let voice = |group, channel, voice| ChannelVoice {
        group,
        channel,
        voice,
    };
    let cc = |controller, value| voice(0, 1, Voice::Midi1ControlChange { controller, value });
    let mut voices = Vec::new();
    for value in [0, 64, 127] {
        voices.extend((1..=5).map(|controller| cc(controller, value)));
        voices.extend([cc(99, 2), cc(98, 44), cc(6, value), cc(38, value)]);
        voices.extend([cc(101, 0), cc(100, 0), cc(6, value)]);
        voices.push(voice(
            0,
            1,
            Voice::Midi1NoteOn {
                note: 60,
                velocity: value,
            },
        ));
        voices.push(voice(
            0,
            1,
            Voice::Midi1NoteOff {
                note: 60,
                velocity: 0,
            },
        ));
    }
    voices.push(voice(
        0,
        1,
        Voice::Midi2ControlChange {
            controller: 2,
            value: u32::MAX,
        },
    ));
    voices.push(voice(
        0,
        1,
        Voice::Midi2AssignableController {
            bank: 2,
            index: 45,
            value: 1 << 30,
        },
    ));
    voices.push(voice(
        0,
        1,
        Voice::Midi2NoteOn {
            note: 61,
            velocity: 0,
        },
    ));
    let mut events = EventList::with_capacity(voices.len() + 1);
    for (offset, voice) in voices.iter().enumerate() {
        events.push(voice.event(offset as u32));
    }
    events.push(Event {
        offset: 0,
        words: [0x3016_7E7F, 0x0D70_0201, 0, 0],
    });
    events
}

#[test]
fn dispatch_neither_allocates_nor_frees() {
    let set = set();
    let ids: Vec<_> = set.ids().collect();
    let (mut learn, mut audio) = Learn::new(&set, Config::default());
    let events = events();

    assert_no_allocations("a block with nothing mapped", || audio.process(&events));

    for (index, &id) in ids.iter().enumerate() {
        learn.map(id, Mapping::new(source(index))).expect("legal");
    }
    assert_no_allocations("the first block after a publish", || audio.process(&events));
    assert_no_allocations("warm blocks", || {
        for _ in 0..64 {
            audio.process(&events);
        }
    });

    // Every buffer of the snapshot has held a table by now; each publish
    // drops one on this thread, outside the gate, and the block after
    // takes the new one without freeing the old.
    for &id in &ids[..4] {
        learn.unmap(id).expect("mapped");
        assert_no_allocations("a block after a later publish", || audio.process(&events));
    }
    // The gate ran real dispatch: every target was reached.
    let moved = ids.iter().filter(|&&id| set.value(id) != 1_000.0).count();
    assert_eq!(moved, TARGETS);

    // Staging a capture while armed allocates nothing either.
    learn
        .arm(ids[0], Takes::CONTROLLER | Takes::NOTE)
        .expect("known");
    assert_no_allocations("armed blocks", || {
        for _ in 0..8 {
            audio.process(&events);
        }
    });
    assert!(learn.poll().is_some(), "the gate ran real capture");
}

#[test]
fn capturing_with_reservations_neither_allocates_nor_frees() {
    // A plugin that reserves controllers makes every captured controller
    // check them, on the audio thread.
    let set = set();
    let ids: Vec<_> = set.ids().collect();
    let mut config = Config::default();
    config.reserved_ccs = vec![
        CcReservation::new(Some(0), Some(1), 4, 5).expect("legal"),
        CcReservation::new(None, None, 90, 100).expect("legal"),
    ];
    config.reserved_nrpns = vec![NrpnReservation::new(None, Some(2), 5_000, 5_010).expect("legal")];
    let (mut learn, mut audio) = Learn::new(&set, config);
    let events = events();

    learn
        .arm(ids[0], Takes::CONTROLLER | Takes::NOTE)
        .expect("known");
    assert_no_allocations("armed blocks with reservations", || {
        for _ in 0..8 {
            audio.process(&events);
        }
    });
    let Some(Learned::Mapped { mapping, .. }) = learn.poll() else {
        panic!("the gate ran real capture");
    };
    assert!(
        !matches!(mapping.source.control, Control::Cc(4 | 5)),
        "a reserved controller was captured: {mapping:?}"
    );
}

/// A target that counts what it is told, allocating nothing.
struct Counting;

impl Target for Counting {
    fn begin(&mut self) {}
    fn apply(&mut self, normalized: f64) {
        std::hint::black_box(normalized);
    }
    fn end(&mut self) {}
}

#[test]
fn staging_ui_targets_and_polling_neither_allocate_nor_free() {
    // The audio thread stages, and a poll tick, busy or idle,
    // allocates nothing.
    let set = set();
    let (mut learn, mut audio) = Learn::new(&set, Config::default());
    for index in 0..16 {
        let key: &'static str = Box::leak(format!("custom{index}").into_boxed_str());
        let id = learn.register(key, Box::new(Counting)).expect("free");
        learn.map(id, Mapping::new(source(index))).expect("legal");
    }
    let events = events();
    assert_no_allocations("the first block after a publish", || audio.process(&events));
    assert_no_allocations("warm blocks and busy polls", || {
        for _ in 0..8 {
            audio.process(&events);
            assert!(learn.poll().is_none());
        }
    });
    assert_no_allocations("idle polls", || {
        for _ in 0..32 {
            assert!(learn.poll().is_none());
        }
    });
}
