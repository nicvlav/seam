//! The UMP crate's audio-thread calls neither allocate nor free: a block's
//! list filled past its capacity, read, changed in place and cleared, words
//! split into packets, and channel voice messages decoded and encoded.

seam_testkit::install_alloc_gate!();

use seam_testkit::alloc::assert_no_allocations;
use seam_ump::{ChannelVoice, Event, EventList, Packets, Voice, packet_words};

const CAPACITY: usize = 256;

/// Every channel voice message Seam reads, on two groups and two channels.
fn voices() -> Vec<ChannelVoice> {
    let messages = [
        Voice::Midi1NoteOff {
            note: 60,
            velocity: 0,
        },
        Voice::Midi1NoteOn {
            note: 60,
            velocity: 127,
        },
        Voice::Midi1ControlChange {
            controller: 7,
            value: 100,
        },
        Voice::Midi2NoteOff {
            note: 60,
            velocity: 0,
        },
        Voice::Midi2NoteOn {
            note: 60,
            velocity: u16::MAX,
        },
        Voice::Midi2ControlChange {
            controller: 7,
            value: u32::MAX,
        },
        Voice::Midi2RegisteredController {
            bank: 0,
            index: 1,
            value: 1,
        },
        Voice::Midi2AssignableController {
            bank: 2,
            index: 44,
            value: 1 << 31,
        },
    ];
    let mut voices = Vec::new();
    for group in [0, 15] {
        for channel in [0, 9] {
            voices.extend(messages.map(|voice| ChannelVoice {
                group,
                channel,
                voice,
            }));
        }
    }
    voices
}

#[test]
fn filling_reading_and_clearing_a_list_does_not_allocate() {
    // Past the capacity, so the full list's drop count runs too.
    let events: Vec<Event> = voices()
        .iter()
        .cycle()
        .take(CAPACITY + 8)
        .zip(0..)
        .map(|(voice, offset)| voice.event(offset))
        .collect();
    let mut list = EventList::with_capacity(CAPACITY);

    let (len, dropped) = assert_no_allocations("an event list filled, read and cleared", || {
        let mut seen = (0, 0);
        for _ in 0..4 {
            list.clear();
            for &event in &events {
                list.push(event);
            }
            for event in list.as_mut_slice() {
                event.offset += 64;
            }
            let mut sum = 0_u64;
            for event in list.as_slice() {
                sum += u64::from(event.message_type()) + u64::from(event.group());
                if let Some(voice) = event.channel_voice() {
                    sum += u64::from(voice.event(event.offset).words[0]);
                }
            }
            list.add_dropped(1);
            std::hint::black_box(sum);
            seen = (list.as_slice().len(), list.dropped());
        }
        seen
    });
    assert_eq!((len, dropped), (CAPACITY, 9), "filled to capacity, past it");
}

#[test]
fn splitting_words_and_decoding_and_encoding_voices_does_not_allocate() {
    // Every voice as its packet's words, packets of the other sizes Seam
    // skips, and a MIDI 2.0 note-on cut after its first word at the end.
    let mut words = Vec::new();
    for voice in voices() {
        let packet = voice.words();
        let length = packet_words(voice.event(0).message_type()).expect("a type");
        words.extend_from_slice(&packet[..length]);
    }
    words.extend([0x0000_0000, 0xB000_0000, 0, 0, 0x5000_0000, 0, 0, 0]);
    words.push(0x4090_3C00);

    let (voices, cut) = assert_no_allocations("packets split, decoded and encoded", || {
        let (mut voices, mut cut) = (0, 0);
        for packet in Packets::new(&words, 32) {
            match packet {
                Ok(event) => {
                    if let Some(voice) = ChannelVoice::decode(event.words) {
                        assert_eq!(voice.words(), event.words, "encodes back");
                        voices += 1;
                    }
                }
                Err(truncated) => cut += truncated.needed - truncated.available,
            }
        }
        (voices, cut)
    });
    assert_eq!((voices, cut), (32, 1));
}
