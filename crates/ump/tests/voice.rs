//! Reading and writing channel voice packets.

use proptest::prelude::*;
use proptest::test_runner::FileFailurePersistence;
use seam_ump::{ChannelVoice, Event, Voice};

#[test]
fn known_packets_read_as_their_messages() {
    let cases = [
        (
            [0x2BB3_077F, 0, 0, 0],
            11,
            3,
            Voice::Midi1ControlChange {
                controller: 7,
                value: 127,
            },
        ),
        (
            [0x2090_3C00, 0, 0, 0],
            0,
            0,
            Voice::Midi1NoteOn {
                note: 60,
                velocity: 0,
            },
        ),
        (
            [0x2081_3C40, 0, 0, 0],
            0,
            1,
            Voice::Midi1NoteOff {
                note: 60,
                velocity: 64,
            },
        ),
        (
            [0x4F9F_4003, 0x8000_1234, 0, 0],
            15,
            15,
            Voice::Midi2NoteOn {
                note: 64,
                velocity: 0x8000,
            },
        ),
        (
            [0x4180_4000, 0xFFFF_0000, 0, 0],
            1,
            0,
            Voice::Midi2NoteOff {
                note: 64,
                velocity: 0xFFFF,
            },
        ),
        (
            [0x40B2_0100, 0x8000_0000, 0, 0],
            0,
            2,
            Voice::Midi2ControlChange {
                controller: 1,
                value: 0x8000_0000,
            },
        ),
        (
            [0x4020_0006, 0x1000_0000, 0, 0],
            0,
            0,
            Voice::Midi2RegisteredController {
                bank: 0,
                index: 6,
                value: 0x1000_0000,
            },
        ),
        (
            [0x4235_027F, 0xFFFF_FFFF, 0, 0],
            2,
            5,
            Voice::Midi2AssignableController {
                bank: 2,
                index: 127,
                value: u32::MAX,
            },
        ),
    ];
    for (words, group, channel, voice) in cases {
        let expected = ChannelVoice {
            group,
            channel,
            voice,
        };
        let event = Event { offset: 9, words };
        assert_eq!(event.channel_voice(), Some(expected), "{words:08X?}");
        assert_eq!(event.group(), group);
    }
}

#[test]
fn spec_vectors_encode_and_decode() {
    // Words laid out by hand from the UMP field tables, not produced by the
    // encoder: message type, group, status, channel and two data bytes in the
    // first word; MIDI 2.0's data in the second.
    let vectors = [
        // MIDI 1.0 note-on, group 0, channel 0, note 60, velocity 127.
        (
            0,
            0,
            Voice::Midi1NoteOn {
                note: 60,
                velocity: 127,
            },
            [0x2090_3C7F, 0],
        ),
        // MIDI 1.0 note-off, group 1, channel 2, note 0x40, velocity 0x10.
        (
            1,
            2,
            Voice::Midi1NoteOff {
                note: 0x40,
                velocity: 0x10,
            },
            [0x2182_4010, 0],
        ),
        // MIDI 1.0 control change, group 15, channel 15, controller 127,
        // value 127.
        (
            15,
            15,
            Voice::Midi1ControlChange {
                controller: 127,
                value: 127,
            },
            [0x2FBF_7F7F, 0],
        ),
        // MIDI 2.0 note-on, group 0, channel 0, note 60, velocity 0xC000,
        // attribute type 0 with no attribute data.
        (
            0,
            0,
            Voice::Midi2NoteOn {
                note: 60,
                velocity: 0xC000,
            },
            [0x4090_3C00, 0xC000_0000],
        ),
        // MIDI 2.0 note-off, group 3, channel 4, note 0x30, velocity 0x1234.
        (
            3,
            4,
            Voice::Midi2NoteOff {
                note: 0x30,
                velocity: 0x1234,
            },
            [0x4384_3000, 0x1234_0000],
        ),
        // MIDI 2.0 control change, group 0, channel 1, controller 7,
        // value 0x80000000.
        (
            0,
            1,
            Voice::Midi2ControlChange {
                controller: 7,
                value: 0x8000_0000,
            },
            [0x40B1_0700, 0x8000_0000],
        ),
        // MIDI 2.0 registered controller, group 2, channel 3, bank 0, index 2.
        (
            2,
            3,
            Voice::Midi2RegisteredController {
                bank: 0,
                index: 2,
                value: 0x1234_5678,
            },
            [0x4223_0002, 0x1234_5678],
        ),
        // MIDI 2.0 assignable controller, group 0, channel 0, bank 1,
        // index 0x23.
        (
            0,
            0,
            Voice::Midi2AssignableController {
                bank: 1,
                index: 0x23,
                value: u32::MAX,
            },
            [0x4030_0123, 0xFFFF_FFFF],
        ),
    ];
    for (group, channel, voice, [first, second]) in vectors {
        let message = ChannelVoice {
            group,
            channel,
            voice,
        };
        assert_eq!(message.words(), [first, second, 0, 0], "{voice:?}");
        assert_eq!(ChannelVoice::decode([first, second, 0, 0]), Some(message));
    }
    // The data bytes' top bits are not data: a reader masks them.
    assert_eq!(
        ChannelVoice::decode([0x2090_BCFF, 0, 0, 0]).map(|message| message.voice),
        Some(Voice::Midi1NoteOn {
            note: 0x3C,
            velocity: 0x7F
        })
    );
}

#[test]
fn other_messages_read_as_nothing() {
    let others = [
        [0x0000_0000, 0, 0, 0],           // utility: no-op
        [0x1000_F800, 0, 0, 0],           // system real time: clock
        [0x20E0_0040, 0, 0, 0],           // MIDI 1.0 pitch bend
        [0x20C0_0500, 0, 0, 0],           // MIDI 1.0 program change
        [0x3016_7E7F, 0x0D70_0201, 0, 0], // SysEx7
        [0x40E0_0000, 0x8000_0000, 0, 0], // MIDI 2.0 pitch bend
        // MIDI 2.0 relative registered controller
        [0x4040_0001, 0x0000_0001, 0, 0],
        // MIDI 2.0 relative assignable controller
        [0x4050_0001, 0x0000_0001, 0, 0],
        // MIDI 2.0 registered per-note controller
        [0x4000_3C01, 0x8000_0000, 0, 0],
    ];
    for words in others {
        assert_eq!(ChannelVoice::decode(words), None, "{words:08X?}");
    }
}

fn voice() -> impl Strategy<Value = Voice> {
    let seven = 0_u8..=127;
    prop_oneof![
        (seven.clone(), seven.clone())
            .prop_map(|(note, velocity)| Voice::Midi1NoteOff { note, velocity }),
        (seven.clone(), seven.clone())
            .prop_map(|(note, velocity)| Voice::Midi1NoteOn { note, velocity }),
        (seven.clone(), seven.clone())
            .prop_map(|(controller, value)| Voice::Midi1ControlChange { controller, value }),
        (seven.clone(), any::<u16>())
            .prop_map(|(note, velocity)| Voice::Midi2NoteOff { note, velocity }),
        (seven.clone(), any::<u16>())
            .prop_map(|(note, velocity)| Voice::Midi2NoteOn { note, velocity }),
        (seven.clone(), any::<u32>())
            .prop_map(|(controller, value)| Voice::Midi2ControlChange { controller, value }),
        (seven.clone(), seven.clone(), any::<u32>()).prop_map(|(bank, index, value)| {
            Voice::Midi2RegisteredController { bank, index, value }
        }),
        (seven.clone(), seven, any::<u32>()).prop_map(|(bank, index, value)| {
            Voice::Midi2AssignableController { bank, index, value }
        }),
    ]
}

proptest! {
    #![proptest_config(ProptestConfig {
        failure_persistence: Some(Box::new(FileFailurePersistence::Direct(
            "tests/voice.proptest-regressions",
        ))),
        ..ProptestConfig::default()
    })]

    /// Every message Seam reads survives writing and reading back.
    #[test]
    fn a_written_message_reads_back(group in 0_u8..16, channel in 0_u8..16, voice in voice()) {
        let message = ChannelVoice { group, channel, voice };
        prop_assert_eq!(ChannelVoice::decode(message.words()), Some(message));
        prop_assert_eq!(message.event(4).offset, 4);
    }

    /// Reading any packet never panics, and whatever it reads writes back
    /// as a packet that reads the same.
    #[test]
    fn any_packet_reads_without_panicking(words in any::<[u32; 4]>()) {
        if let Some(message) = ChannelVoice::decode(words) {
            prop_assert_eq!(ChannelVoice::decode(message.words()), Some(message));
        }
    }
}
