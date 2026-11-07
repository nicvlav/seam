//! Instruction counts for learn's dispatch of one block of MIDI on the audio
//! thread, with a table of mappings published. Run by CI with iai-callgrind,
//! which compares them with the pull request's base.

#![allow(missing_docs, reason = "the benchmark macros generate public items")]

use std::hint::black_box;

use iai_callgrind::{library_benchmark, library_benchmark_group, main};
use seam_learn::{Config, Control, Learn, LearnAudio, Mapping, Source};
use seam_params::{Kind, ParamDecl, ParamSetBuilder, Range};
use seam_ump::{ChannelVoice, Event, EventList, Voice};

const TARGETS: usize = 64;

/// A learn with every target mapped to a controller or a note, and the
/// audio side that dispatches.
fn mapped() -> (Learn, LearnAudio) {
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
    let set = builder.build();
    let (mut learn, audio) = Learn::new(&set, Config::default());
    for (index, id) in set.ids().enumerate() {
        let control = match index % 3 {
            0 => Control::Cc([1, 2, 3, 4, 5, 7, 8, 9][index / 3 % 8]),
            1 => Control::Nrpn(300 + (index / 3 % 4) as u16),
            _ => Control::Note(60 + (index / 3 % 4) as u8),
        };
        let source = Source {
            control,
            group: None,
            channel: Some(1),
        };
        learn.map(id, Mapping::new(source)).expect("legal");
    }
    (learn, audio)
}

/// A block of controller moves, an NRPN with its data, and notes.
fn block() -> EventList {
    let voice = |voice| ChannelVoice {
        group: 0,
        channel: 1,
        voice,
    };
    let cc = |controller, value| voice(Voice::Midi1ControlChange { controller, value });
    let mut voices = Vec::new();
    for value in [0, 64, 127] {
        voices.extend((1..=8).map(|controller| cc(controller, value)));
        voices.extend([cc(99, 2), cc(98, 44), cc(6, value), cc(38, value)]);
        voices.push(voice(Voice::Midi1NoteOn {
            note: 60,
            velocity: value.max(1),
        }));
        voices.push(voice(Voice::Midi1NoteOff {
            note: 60,
            velocity: 0,
        }));
    }
    let mut events = EventList::with_capacity(voices.len());
    for (offset, voice) in voices.iter().enumerate() {
        events.push(voice.event(offset as u32));
    }
    events.push(Event {
        offset: 0,
        words: [0x3016_7E7F, 0x0D70_0201, 0, 0],
    });
    events
}

#[library_benchmark]
#[bench::first_block(mapped(), block())]
fn dispatch(state: (Learn, LearnAudio), events: EventList) {
    let (_learn, mut audio) = state;
    audio.process(black_box(&events));
}

library_benchmark_group!(name = learn; benchmarks = dispatch);
main!(library_benchmark_groups = learn);
