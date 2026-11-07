//! Packet sizes and splitting a stream of words.

use proptest::prelude::*;
use proptest::test_runner::FileFailurePersistence;
use seam_ump::{Packets, Truncated, packet_words};

/// The UMP 1.1 table of message types: the type and the packet's size in
/// bits. Types 0x6 to 0xE are reserved or, for 0xD, flex data; a reserved
/// type still has a size.
const TABLE: [(u8, usize); 16] = [
    (0x0, 32),  // utility
    (0x1, 32),  // system real time and common
    (0x2, 32),  // MIDI 1.0 channel voice
    (0x3, 64),  // SysEx7
    (0x4, 64),  // MIDI 2.0 channel voice
    (0x5, 128), // SysEx8 and mixed data set
    (0x6, 32),  // reserved
    (0x7, 32),  // reserved
    (0x8, 64),  // reserved
    (0x9, 64),  // reserved
    (0xA, 64),  // reserved
    (0xB, 96),  // reserved
    (0xC, 96),  // reserved
    (0xD, 128), // flex data
    (0xE, 128), // reserved
    (0xF, 128), // UMP stream
];

#[test]
fn every_message_type_has_the_size_the_table_gives() {
    for (message_type, bits) in TABLE {
        assert_eq!(
            packet_words(message_type),
            Some(bits / 32),
            "{message_type:#X}"
        );
    }
    for message_type in 16..=u8::MAX {
        assert_eq!(packet_words(message_type), None, "{message_type:#X}");
    }
}

#[test]
fn a_stream_splits_into_its_packets() {
    let stream = [
        0x2090_3C7F, // MIDI 1.0 note-on
        0x4090_3C00,
        0xC000_0000, // MIDI 2.0 note-on
        0x0000_0000, // utility no-op
        0x3016_7E7F,
        0x0D70_0201, // SysEx7
        0xF000_0000,
        0,
        0,
        0, // stream message
        0xB000_0001,
        0x0000_0002,
        0x0000_0003, // reserved, 96 bits
    ];
    let packets: Vec<_> = Packets::new(&stream, 7)
        .map(|packet| packet.expect("whole"))
        .collect();
    let words: Vec<[u32; 4]> = packets.iter().map(|event| event.words).collect();
    assert_eq!(
        words,
        [
            [0x2090_3C7F, 0, 0, 0],
            [0x4090_3C00, 0xC000_0000, 0, 0],
            [0, 0, 0, 0],
            [0x3016_7E7F, 0x0D70_0201, 0, 0],
            [0xF000_0000, 0, 0, 0],
            [0xB000_0001, 2, 3, 0],
        ]
    );
    assert!(packets.iter().all(|event| event.offset == 7));
}

#[test]
fn an_empty_stream_has_no_packets() {
    assert_eq!(Packets::new(&[], 0).next(), None);
}

#[test]
fn a_cut_packet_is_refused_once_and_nothing_follows() {
    // The stream, how many whole packets come first, and what is cut.
    let cases = [
        (vec![0x2090_3C7F, 0x4090_3C00], 1, (4, 2, 1)),
        (vec![0x5000_0000, 1, 2], 0, (5, 4, 3)),
        (vec![0xC000_0000, 1], 0, (0xC, 3, 2)),
        (vec![0x0000_0000, 0x0000_0000, 0x3000_0000], 2, (3, 2, 1)),
    ];
    for (stream, whole, (message_type, needed, available)) in cases {
        let mut packets = Packets::new(&stream, 0);
        for _ in 0..whole {
            assert!(packets.next().expect("a packet").is_ok());
        }
        let cut: Truncated = packets.next().expect("the cut packet").expect_err("cut");
        assert_eq!(
            (cut.message_type, cut.needed, cut.available),
            (message_type, needed, available)
        );
        assert_eq!(packets.next(), None, "nothing after a cut packet");
    }
}

proptest! {
    #![proptest_config(ProptestConfig {
        failure_persistence: Some(Box::new(FileFailurePersistence::Direct(
            "tests/packets.proptest-regressions",
        ))),
        ..ProptestConfig::default()
    })]

    /// Packets written one after another split back apart, whatever their
    /// types: the sizes alone frame a stream.
    #[test]
    fn a_stream_of_whole_packets_splits_back(
        packets in proptest::collection::vec((0_u8..16, any::<[u32; 3]>()), 0..24),
    ) {
        let mut stream = Vec::new();
        let mut expected = Vec::new();
        for (message_type, rest) in packets {
            let size = TABLE[usize::from(message_type)].1 / 32;
            let first = u32::from(message_type) << 28 | (rest[0] & 0x0FFF_FFFF);
            let mut words = [0; 4];
            words[0] = first;
            words[1..size].copy_from_slice(&rest[..size - 1]);
            stream.extend_from_slice(&words[..size]);
            expected.push(words);
        }
        let split: Result<Vec<_>, _> = Packets::new(&stream, 0).collect();
        let split: Vec<[u32; 4]> = split.expect("whole packets").iter().map(|e| e.words).collect();
        prop_assert_eq!(split, expected);
    }

    /// Any words at all split without panicking, and the packets plus the
    /// cut tail account for every word.
    #[test]
    fn any_words_split_without_losing_one(stream in proptest::collection::vec(any::<u32>(), 0..64)) {
        let mut taken = 0;
        for packet in Packets::new(&stream, 0) {
            match packet {
                Ok(event) => {
                    taken += packet_words(event.message_type()).expect("a type");
                }
                Err(cut) => {
                    prop_assert_eq!(taken + cut.available, stream.len());
                    prop_assert!(cut.available < cut.needed);
                    taken = stream.len();
                }
            }
        }
        prop_assert_eq!(taken, stream.len());
    }
}
