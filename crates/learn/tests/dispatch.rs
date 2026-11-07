//! The audio path of learn: parsing, lookup, scaling and curves.

use std::sync::Arc;

use seam_learn::{
    Config, Control, Curve, CurveError, Learn, LearnAudio, LearnError, Mapping, NrpnReservation,
    ReservationError, Source,
};
use seam_params::{Kind, ParamDecl, ParamId, ParamSet, ParamSetBuilder};
use seam_ump::{ChannelVoice, Event, EventList, Voice};

/// A value no message in these tests writes.
const UNTOUCHED: f32 = 0.37;

fn set_of(count: usize) -> Arc<ParamSet> {
    let mut builder = ParamSetBuilder::new();
    for index in 0..count {
        let key: &'static str = Box::leak(format!("p{index}").into_boxed_str());
        builder
            .add(ParamDecl::new(
                key,
                key,
                Kind::continuous(0.0, 1.0, 0.0, None),
                UNTOUCHED,
            ))
            .expect("valid");
    }
    builder.build()
}

struct Rig {
    set: Arc<ParamSet>,
    learn: Learn,
    audio: LearnAudio,
    ids: Vec<ParamId>,
}

fn rig(count: usize) -> Rig {
    rig_with(count, Config::default())
}

fn rig_with(count: usize, config: Config) -> Rig {
    let set = set_of(count);
    let (learn, audio) = Learn::new(&set, config);
    let ids = set.ids().collect();
    Rig {
        set,
        learn,
        audio,
        ids,
    }
}

impl Rig {
    /// Runs one block of `voices` on group 0, channel 0 unless they say
    /// otherwise.
    fn block(&mut self, voices: &[ChannelVoice]) {
        let mut events = EventList::with_capacity(voices.len());
        for (offset, voice) in voices.iter().enumerate() {
            events.push(voice.event(offset as u32));
        }
        self.audio.process(&events);
    }

    fn value(&self, index: usize) -> f32 {
        self.set.value(self.ids[index])
    }

    fn map(&mut self, index: usize, source: Source) {
        self.learn
            .map(self.ids[index], Mapping::new(source))
            .expect("a legal mapping");
    }
}

/// What a learned write of `normalized` leaves in a parameter that runs 0 to
/// 1 with no step: the same number, as the `f32` the store holds.
fn stored(normalized: f64) -> f32 {
    normalized.clamp(0.0, 1.0) as f32
}

fn on(group: u8, channel: u8, voice: Voice) -> ChannelVoice {
    ChannelVoice {
        group,
        channel,
        voice,
    }
}

fn cc(controller: u8, value: u8) -> ChannelVoice {
    on(0, 0, Voice::Midi1ControlChange { controller, value })
}

/// MIDI 1.0 values scale centre-anchored.
fn seven(value: u8) -> f64 {
    let value = f64::from(value);
    if value <= 64.0 {
        value / 128.0
    } else {
        0.5 + (value - 64.0) / 126.0
    }
}

fn fourteen(value: u16) -> f64 {
    let value = f64::from(value);
    if value <= 8192.0 {
        value / 16384.0
    } else {
        0.5 + (value - 8192.0) / 16382.0
    }
}

/// The two controllers of MIDI 1.0 NRPN `number`'s address, 99 and 98.
fn nrpn_address(number: u16) -> [ChannelVoice; 2] {
    [cc(99, (number >> 7) as u8), cc(98, (number & 0x7F) as u8)]
}

#[test]
fn a_mapped_controller_writes_its_target_in_the_block_it_arrives_in() {
    let mut rig = rig(2);
    rig.map(0, Source::any(Control::Cc(7)));

    for value in [127, 64, 0, 100] {
        rig.block(&[cc(7, value)]);
        assert_eq!(rig.value(0), stored(seven(value)), "CC 7 = {value}");
    }
    assert_eq!(rig.value(1), UNTOUCHED);
    assert_eq!(rig.value(0), stored(seven(100)));
    // CC 64 is the centre, 0.5, not 64 / 127.
    rig.block(&[cc(7, 64)]);
    assert_eq!(rig.value(0), 0.5);
}

#[test]
fn only_mapped_targets_take_a_slot() {
    let mut rig = rig(8);
    assert_eq!(rig.audio.refresh_slots(), 0);
    rig.map(3, Source::any(Control::Cc(1)));
    assert_eq!(rig.audio.refresh_slots(), 1);
    assert_eq!(rig.learn.len(), 1);
    rig.learn.unmap(rig.ids[3]).expect("mapped");
    assert_eq!(rig.audio.refresh_slots(), 0);
    assert!(rig.learn.is_empty());
    assert_eq!(rig.learn.unmap(rig.ids[3]), None);
}

#[test]
fn one_source_drives_every_target_mapped_to_it_and_a_target_has_one() {
    let mut rig = rig(3);
    rig.map(0, Source::any(Control::Cc(7)));
    rig.map(1, Source::any(Control::Cc(7)));
    rig.map(2, Source::any(Control::Cc(8)));
    rig.block(&[cc(7, 127)]);
    assert_eq!(
        [rig.value(0), rig.value(1), rig.value(2)],
        [1.0, 1.0, UNTOUCHED]
    );

    // A new mapping replaces the target's source.
    let replaced = rig
        .learn
        .map(rig.ids[0], Mapping::new(Source::any(Control::Cc(9))))
        .expect("legal");
    assert_eq!(replaced, Some(Mapping::new(Source::any(Control::Cc(7)))));
    rig.block(&[cc(7, 0)]);
    assert_eq!([rig.value(0), rig.value(1)], [1.0, 0.0]);
    rig.block(&[cc(9, 0)]);
    assert_eq!(rig.value(0), 0.0);
    assert_eq!(rig.learn.len(), 3);
}

#[test]
fn messages_apply_in_order_so_the_block_ends_on_the_last() {
    let mut rig = rig(1);
    rig.map(0, Source::any(Control::Cc(7)));
    rig.block(&[cc(7, 10), cc(7, 127), cc(7, 33)]);
    assert_eq!(rig.value(0), stored(seven(33)));
}

#[test]
fn every_packet_reaches_the_processor_untouched_and_in_order() {
    // Learn reads the block's MIDI and never filters it. MIDI-CI rides on
    // SysEx7.
    let mut rig = rig(2);
    rig.map(0, Source::any(Control::Cc(7)));
    rig.map(1, Source::any(Control::Nrpn(300)));
    let mut events = EventList::with_capacity(8);
    let packets = [
        cc(7, 100).event(0),
        Event {
            offset: 1,
            words: [0x3016_7E7F, 0x0D70_0201, 0, 0],
        },
        nrpn_address(300)[0].event(2),
        nrpn_address(300)[1].event(2),
        cc(6, 9).event(3),
        on(
            3,
            2,
            Voice::Midi2NoteOn {
                note: 60,
                velocity: 1,
            },
        )
        .event(4),
        Event {
            offset: 5,
            words: [0x3036_0102, 0x0304_0506, 0, 0],
        },
    ];
    for packet in packets {
        events.push(packet);
    }
    let before = events.as_slice().to_vec();
    rig.audio.process(&events);
    assert_eq!(events.as_slice(), &before[..]);
    assert_eq!(rig.value(0), stored(seven(100)));
    assert_eq!(rig.value(1), stored(fourteen(9 << 7)));
}

#[test]
fn groups_and_channels_stay_apart_and_any_matches_every_one() {
    let mut rig = rig(5);
    let sources = [
        (Some(0), Some(0)),
        (Some(1), Some(0)),
        (None, Some(0)),
        (Some(0), None),
        (None, None),
    ];
    for (index, (group, channel)) in sources.into_iter().enumerate() {
        rig.map(
            index,
            Source {
                control: Control::Cc(7),
                group,
                channel,
            },
        );
    }
    rig.block(&[on(
        1,
        0,
        Voice::Midi1ControlChange {
            controller: 7,
            value: 127,
        },
    )]);
    let values: Vec<f32> = (0..5).map(|index| rig.value(index)).collect();
    assert_eq!(values, [UNTOUCHED, 1.0, 1.0, UNTOUCHED, 1.0]);

    rig.block(&[on(
        0,
        5,
        Voice::Midi1ControlChange {
            controller: 7,
            value: 0,
        },
    )]);
    let values: Vec<f32> = (0..5).map(|index| rig.value(index)).collect();
    assert_eq!(values, [UNTOUCHED, 1.0, 1.0, 0.0, 0.0]);
}

#[test]
fn an_nrpn_assembles_and_a_high_byte_alone_reads_as_midi_1_says() {
    // A high byte resets the low byte and dispatches at once, as `msb << 7`;
    // a low byte refines it, here a block later.
    let mut rig = rig(1);
    rig.map(0, Source::any(Control::Nrpn(300)));
    let [high, low] = nrpn_address(300);
    rig.block(&[high, low, cc(6, 127)]);
    assert_eq!(rig.value(0), stored(fourteen(127 << 7)));
    assert!(
        rig.value(0) < 1.0,
        "a 14-bit binding tops out below 1 on high bytes alone"
    );
    rig.block(&[cc(38, 127)]);
    assert_eq!(rig.value(0), 1.0);
    // A new high byte resets the low byte.
    rig.block(&[cc(6, 64)]);
    assert_eq!(rig.value(0), 0.5);
}

#[test]
fn a_coarse_binding_reads_a_high_byte_as_seven_bit_and_ignores_low_bytes() {
    let mut rig = rig(1);
    let mapping = Mapping::new(Source::any(Control::Nrpn(300))).as_coarse();
    rig.learn.map(rig.ids[0], mapping).expect("legal");
    let [high, low] = nrpn_address(300);
    rig.block(&[high, low, cc(6, 127)]);
    assert_eq!(rig.value(0), 1.0);
    rig.block(&[cc(38, 0)]);
    assert_eq!(rig.value(0), 1.0, "the low byte is ignored");
    rig.block(&[cc(6, 64)]);
    assert_eq!(rig.value(0), 0.5);
}

#[test]
fn a_partial_nrpn_sequence_is_dropped() {
    let mut rig = rig(2);
    rig.map(0, Source::any(Control::Nrpn(44)));
    rig.map(1, Source::any(Control::Nrpn(300)));
    // An address with no high byte yet: NRPN 44 needs both.
    rig.block(&[cc(98, 44), cc(6, 100), cc(38, 5)]);
    assert_eq!([rig.value(0), rig.value(1)], [UNTOUCHED, UNTOUCHED]);

    let [high, low] = nrpn_address(300);
    rig.block(&[high, low, cc(6, 100)]);
    assert_eq!(rig.value(1), stored(fourteen(100 << 7)));
    // A new address clears the data entry, so a low byte alone is partial.
    let [high, low] = nrpn_address(300);
    rig.block(&[high, low, cc(38, 5)]);
    assert_eq!(rig.value(1), stored(fourteen(100 << 7)));
}

#[test]
fn rpn_data_entry_never_dispatches_as_the_last_nrpn() {
    // RPN data entry must not be read as the last NRPN's value.
    let mut rig = rig(1);
    rig.map(0, Source::any(Control::Nrpn(300)));
    let [high, low] = nrpn_address(300);
    rig.block(&[high, low, cc(6, 10)]);
    let before = rig.value(0);
    // RPN 0, pitch bend range, then its data entry.
    rig.block(&[cc(101, 0), cc(100, 0), cc(6, 2), cc(38, 0)]);
    assert_eq!(rig.value(0), before);
    // Selecting the NRPN again, both bytes, routes data entry to it again.
    rig.block(&[high, low, cc(6, 127), cc(38, 127)]);
    assert_eq!(rig.value(0), 1.0);
}

#[test]
fn the_parser_runs_while_nothing_is_mapped() {
    // An address sent while nothing is mapped is still current later.
    let mut rig = rig(1);
    let [high, low] = nrpn_address(300);
    rig.block(&[high, low]);
    rig.map(0, Source::any(Control::Nrpn(300)));
    rig.block(&[cc(6, 127), cc(38, 127)]);
    assert_eq!(rig.value(0), 1.0);
}

#[test]
fn the_parser_owned_controllers_never_bind() {
    // The parser owns the NRPN and RPN address, data entry, and data
    // increment and decrement controllers.
    let mut rig = rig(1);
    for controller in [6, 38, 96, 97, 98, 99, 100, 101] {
        let source = Source::any(Control::Cc(controller));
        assert_eq!(
            rig.learn.map(rig.ids[0], Mapping::new(source)),
            Err(LearnError::Blocked(source))
        );
    }
    assert!(rig.learn.is_empty());
    // MIDI 2.0 sends parameter numbers as their own messages, so the same
    // controllers there drive nothing either: not a CC binding, and not an
    // NRPN they would assemble in MIDI 1.0.
    let mut rig = self::rig(2);
    rig.map(0, Source::any(Control::Cc(7)));
    rig.map(1, Source::any(Control::Nrpn(300)));
    let midi2 = |controller, value| on(0, 0, Voice::Midi2ControlChange { controller, value });
    rig.block(&[
        midi2(99, 2 << 25),
        midi2(98, 44 << 25),
        midi2(6, u32::MAX),
        midi2(38, u32::MAX),
        midi2(101, 0),
        midi2(100, 0),
    ]);
    assert_eq!([rig.value(0), rig.value(1)], [UNTOUCHED, UNTOUCHED]);
}

#[test]
fn each_group_and_channel_keeps_its_own_nrpn_state() {
    // The parser keeps a fixed state per group and channel, so two
    // controllers sending NRPNs at once never mix addresses.
    let mut rig = rig(3);
    let on_channel = |channel| Source {
        control: Control::Nrpn(300),
        group: Some(0),
        channel: Some(channel),
    };
    rig.map(0, on_channel(0));
    rig.map(1, on_channel(1));
    rig.map(
        2,
        Source {
            control: Control::Nrpn(5),
            group: Some(0),
            channel: Some(1),
        },
    );
    let at = |group, channel, controller, value| {
        on(
            group,
            channel,
            Voice::Midi1ControlChange { controller, value },
        )
    };
    rig.block(&[
        at(0, 0, 99, 2),
        at(0, 0, 98, 44),
        at(0, 1, 99, 0),
        at(0, 1, 98, 5),
        at(0, 0, 6, 127),
        at(0, 0, 38, 127),
    ]);
    assert_eq!(
        [rig.value(0), rig.value(1), rig.value(2)],
        [1.0, UNTOUCHED, UNTOUCHED]
    );

    // An address on one group, data entry on another: dropped.
    let mut rig = self::rig(1);
    rig.map(0, Source::any(Control::Nrpn(300)));
    rig.block(&[at(0, 3, 99, 2), at(0, 3, 98, 44), at(1, 3, 6, 127)]);
    assert_eq!(rig.value(0), UNTOUCHED);
    rig.block(&[at(0, 3, 6, 127), at(0, 3, 38, 127)]);
    assert_eq!(rig.value(0), 1.0);
}

#[test]
fn reset_all_controllers_deselects_the_nrpn() {
    // MIDI 1.0's recommended response to controller 121 sets the
    // parameter number to null; the message still drives a mapping to it.
    let mut rig = rig(2);
    rig.map(0, Source::any(Control::Nrpn(300)));
    rig.map(1, Source::any(Control::Cc(121)));
    let [high, low] = nrpn_address(300);
    rig.block(&[high, low, cc(121, 0), cc(6, 127), cc(38, 127)]);
    assert_eq!([rig.value(0), rig.value(1)], [UNTOUCHED, 0.0]);
    let [high, low] = nrpn_address(300);
    rig.block(&[high, low, cc(6, 127), cc(38, 127)]);
    assert_eq!(rig.value(0), 1.0);
}

#[test]
fn a_reserved_nrpn_is_never_bound() {
    // A reserved NRPN range blocks capture and dispatch. A reservation on
    // one channel blocks a source that could hear that channel.
    let mut config = Config::default();
    config.reserved_nrpns = vec![NrpnReservation::new(None, Some(0), 1000, 1010).expect("valid")];
    let mut rig = rig_with(1, config);
    let target = rig.ids[0];
    for (number, channel) in [(1000, Some(0)), (1005, None), (1010, Some(0))] {
        let source = Source {
            control: Control::Nrpn(number),
            group: Some(2),
            channel,
        };
        assert_eq!(
            rig.learn.map(target, Mapping::new(source)),
            Err(LearnError::Blocked(source))
        );
    }
    let free = [
        Source::any(Control::Nrpn(999)),
        Source::any(Control::Nrpn(1011)),
        Source {
            control: Control::Nrpn(1005),
            group: None,
            channel: Some(1),
        },
        Source::any(Control::Cc(7)),
    ];
    for source in free {
        assert!(
            rig.learn.map(target, Mapping::new(source)).is_ok(),
            "{source:?}"
        );
    }
}

#[test]
fn a_reservation_on_one_group_leaves_the_others_free() {
    let mut config = Config::default();
    config.reserved_nrpns = vec![NrpnReservation::new(Some(0), None, 1000, 1010).expect("valid")];
    let mut rig = rig_with(1, config);
    let target = rig.ids[0];
    let on_group = |group| Source {
        control: Control::Nrpn(1005),
        group,
        channel: Some(4),
    };
    for blocked in [on_group(Some(0)), on_group(None)] {
        assert_eq!(
            rig.learn.map(target, Mapping::new(blocked)),
            Err(LearnError::Blocked(blocked))
        );
    }
    assert!(
        rig.learn
            .map(target, Mapping::new(on_group(Some(1))))
            .is_ok()
    );
}

#[test]
fn a_reservation_reads_its_range_either_way_round_and_refuses_bad_fields() {
    // A backwards range would otherwise reserve nothing.
    let backwards = NrpnReservation::new(None, None, 1010, 1000).expect("valid");
    assert_eq!(backwards.numbers(), 1000..=1010);
    let mut config = Config::default();
    config.reserved_nrpns = vec![backwards];
    let mut rig = rig_with(1, config);
    let source = Source::any(Control::Nrpn(1005));
    assert_eq!(
        rig.learn.map(rig.ids[0], Mapping::new(source)),
        Err(LearnError::Blocked(source))
    );
    assert_eq!(
        NrpnReservation::new(Some(16), None, 0, 1),
        Err(ReservationError::Group)
    );
    assert_eq!(
        NrpnReservation::new(None, Some(16), 0, 1),
        Err(ReservationError::Channel)
    );
    assert_eq!(
        NrpnReservation::new(None, None, 0, 16384),
        Err(ReservationError::Number)
    );
    let full = NrpnReservation::new(Some(15), Some(15), 16383, 0).expect("valid");
    assert_eq!((full.group(), full.channel()), (Some(15), Some(15)));
}

#[test]
fn midi_2_values_read_linearly_and_assignable_controllers_are_nrpns() {
    // MIDI 2.0 values are linear; an assignable controller numbered
    // bank × 128 + index binds as an NRPN.
    let mut rig = rig(2);
    rig.map(0, Source::any(Control::Cc(7)));
    let coarse = Mapping::new(Source::any(Control::Nrpn(300))).as_coarse();
    rig.learn.map(rig.ids[1], coarse).expect("legal");
    for value in [u32::MAX, 0x8000_0000, 0, 12_345_678] {
        let normalized = f64::from(value) / f64::from(u32::MAX);
        rig.block(&[
            on(
                0,
                0,
                Voice::Midi2ControlChange {
                    controller: 7,
                    value,
                },
            ),
            on(
                0,
                0,
                Voice::Midi2AssignableController {
                    bank: 2,
                    index: 44,
                    value,
                },
            ),
        ]);
        assert_eq!(rig.value(0), stored(normalized));
        assert_eq!(
            rig.value(1),
            stored(normalized),
            "the coarse flag is MIDI 1.0's"
        );
    }
    // A registered controller is RPN: never an NRPN.
    rig.block(&[on(
        0,
        0,
        Voice::Midi2RegisteredController {
            bank: 2,
            index: 44,
            value: u32::MAX,
        },
    )]);
    assert_eq!(rig.value(1), stored(12_345_678.0 / f64::from(u32::MAX)));
}

#[test]
fn a_note_on_writes_its_velocity_and_a_note_off_writes_nothing() {
    let mut rig = rig(1);
    rig.map(0, Source::any(Control::Note(60)));
    let midi1_on = |velocity| on(0, 0, Voice::Midi1NoteOn { note: 60, velocity });
    let midi2_on = |velocity| on(0, 0, Voice::Midi2NoteOn { note: 60, velocity });

    rig.block(&[midi1_on(127)]);
    assert_eq!(rig.value(0), 1.0);
    rig.block(&[on(
        0,
        0,
        Voice::Midi1NoteOff {
            note: 60,
            velocity: 0,
        },
    )]);
    rig.block(&[midi1_on(0)]);
    assert_eq!(rig.value(0), 1.0, "MIDI 1.0's velocity 0 is a note-off");
    rig.block(&[midi1_on(64)]);
    assert_eq!(rig.value(0), 0.5);

    rig.block(&[midi2_on(0x8000)]);
    assert_eq!(rig.value(0), stored(f64::from(0x8000_u16) / 65535.0));
    rig.block(&[on(
        0,
        0,
        Voice::Midi2NoteOff {
            note: 60,
            velocity: 0xFFFF,
        },
    )]);
    assert_eq!(rig.value(0), stored(f64::from(0x8000_u16) / 65535.0));
    rig.block(&[midi2_on(0)]);
    assert_eq!(rig.value(0), 0.0, "MIDI 2.0's velocity 0 is a note-on");
    rig.block(&[on(
        0,
        0,
        Voice::Midi1NoteOn {
            note: 61,
            velocity: 99,
        },
    )]);
    assert_eq!(rig.value(0), 0.0, "another key");
}

#[test]
fn a_curve_inverts_then_skews_then_windows() {
    let mut rig = rig(1);
    let curve = Curve::new(2.0, 0.2, 0.6).expect("legal").inverted();
    let mapping = Mapping::new(Source::any(Control::Cc(7))).with_curve(curve);
    rig.learn.map(rig.ids[0], mapping).expect("legal");
    for (value, expected) in [(0, 0.6), (127, 0.2), (64, 0.2 + 0.25 * 0.4)] {
        rig.block(&[cc(7, value)]);
        assert_eq!(rig.value(0), stored(expected), "CC 7 = {value}");
        assert!((curve.apply(seven(value)) - expected).abs() < 1e-7);
    }
}

#[test]
fn curves_are_checked_when_made() {
    for skew in [0.0, -1.0, f32::NAN, f32::INFINITY] {
        assert_eq!(Curve::new(skew, 0.0, 1.0), Err(CurveError::Skew), "{skew}");
    }
    for (low, high) in [
        (-0.1, 1.0),
        (0.0, 1.1),
        (f32::NAN, 1.0),
        (0.0, f32::INFINITY),
    ] {
        assert_eq!(Curve::new(1.0, low, high), Err(CurveError::Window));
    }
    let backwards = Curve::new(1.0, 1.0, 0.0).expect("a window may run backwards");
    assert_eq!(backwards.apply(0.25), 0.75);
    let signed = Curve::new(1.0, -0.0, 1.0).expect("legal");
    assert_eq!(signed.low().to_bits(), 0, "a saved curve never writes -0");
    assert!(Curve::default().is_identity());
    for value in [0.0, 1e-9, 0.25, 0.5, 0.999_999, 1.0] {
        assert_eq!(Curve::IDENTITY.apply(value).to_bits(), f64::to_bits(value));
    }
    assert_eq!(Curve::IDENTITY.apply(-3.0), 0.0);
    assert_eq!(Curve::IDENTITY.apply(7.0), 1.0);
}

#[test]
fn the_ceiling_refuses_a_new_mapping_but_never_a_re_point() {
    let mut config = Config::default();
    config.max_mappings = 2;
    let mut rig = rig_with(3, config);
    rig.map(0, Source::any(Control::Cc(1)));
    rig.map(1, Source::any(Control::Cc(2)));
    let third = Mapping::new(Source::any(Control::Cc(3)));
    assert_eq!(rig.learn.map(rig.ids[2], third), Err(LearnError::Full));
    rig.map(0, Source::any(Control::Cc(4)));
    rig.learn.unmap(rig.ids[1]).expect("mapped");
    assert_eq!(rig.learn.map(rig.ids[2], third), Ok(None));
}

#[test]
fn a_bad_source_or_unknown_target_is_refused_and_changes_nothing() {
    let mut rig = rig(1);
    let bad = [
        Source::any(Control::Cc(128)),
        Source::any(Control::Note(200)),
        Source::any(Control::Nrpn(16384)),
        Source {
            control: Control::Cc(1),
            group: Some(16),
            channel: None,
        },
        Source {
            control: Control::Cc(1),
            group: None,
            channel: Some(16),
        },
    ];
    for source in bad {
        assert_eq!(
            rig.learn.map(rig.ids[0], Mapping::new(source)),
            Err(LearnError::BadSource(source))
        );
    }
    let elsewhere = set_of(4).id_at(3).expect("declared");
    let mapping = Mapping::new(Source::any(Control::Cc(1)));
    assert_eq!(
        rig.learn.map(elsewhere, mapping),
        Err(LearnError::UnknownTarget(elsewhere.into()))
    );
    assert!(rig.learn.is_empty());
    assert_eq!(rig.audio.refresh_slots(), 0);
}

#[test]
fn the_table_at_the_ceiling_still_finds_every_mapping() {
    // The worst case at the default ceiling of 4,096 mappings: every
    // mapping on its own source, spread over every kind and every way of
    // naming a group and channel, then every mapping on one source.
    let count = seam_learn::DEFAULT_MAX_MAPPINGS;
    let mut rig = rig(count + 1);
    let legal_ccs: Vec<u8> = (0..128)
        .filter(|cc| ![6, 38, 96, 97, 98, 99, 100, 101].contains(cc))
        .collect();
    // Sources repeat where a kind runs out of numbers, so some messages
    // drive several targets; each target must still move.
    let source_of = |index: usize| {
        let (kind, n) = (index % 3, index / 3);
        let control = match kind {
            0 => Control::Cc(legal_ccs[n % legal_ccs.len()]),
            1 => Control::Nrpn((n * 7 % 16384) as u16),
            _ => Control::Note((n % 128) as u8),
        };
        let form = n % 4;
        Source {
            control,
            group: (form < 2).then_some((n / 4 % 16) as u8),
            channel: (form % 2 == 0).then_some((n / 64 % 16) as u8),
        }
    };
    for index in 0..count {
        rig.map(index, source_of(index));
    }
    assert_eq!(rig.audio.refresh_slots(), count);
    let extra = Mapping::new(Source::any(Control::Cc(1)));
    assert_eq!(rig.learn.map(rig.ids[count], extra), Err(LearnError::Full));

    for index in 0..count {
        let source = rig.learn.mapping(rig.ids[index]).expect("mapped").source;
        let group = source.group.unwrap_or(15);
        let channel = source.channel.unwrap_or(15);
        let voice = match source.control {
            Control::Cc(controller) => Voice::Midi2ControlChange {
                controller,
                value: u32::MAX,
            },
            Control::Nrpn(number) => Voice::Midi2AssignableController {
                bank: (number >> 7) as u8,
                index: (number & 0x7F) as u8,
                value: u32::MAX,
            },
            Control::Note(note) => Voice::Midi2NoteOn {
                note,
                velocity: u16::MAX,
            },
        };
        rig.block(&[on(group, channel, voice)]);
        assert_eq!(rig.value(index), 1.0, "{source:?}");
    }
    let moved = (0..count).filter(|&index| rig.value(index) == 1.0).count();
    assert_eq!(moved, count);

    // Every target on one source: one message drives all of them.
    for index in 0..count {
        rig.map(index, Source::any(Control::Cc(1)));
    }
    rig.block(&[cc(1, 0)]);
    assert!((0..count).all(|index| rig.value(index) == 0.0));
}

#[test]
fn the_null_parameter_number_selects_nothing() {
    let mut rig = rig(2);
    rig.map(0, Source::any(Control::Nrpn(16383)));
    rig.map(1, Source::any(Control::Nrpn(300)));
    let [high, low] = nrpn_address(300);
    rig.block(&[high, low, cc(6, 50)]);
    let before = rig.value(1);
    // NRPN 127, 127 deselects, so the data entry after it drives nothing.
    rig.block(&[cc(99, 127), cc(98, 127), cc(6, 100), cc(38, 5)]);
    assert_eq!(rig.value(0), UNTOUCHED);
    assert_eq!(rig.value(1), before);
}

#[test]
fn a_data_entry_low_byte_sent_first_waits_for_its_high_byte() {
    let mut rig = rig(1);
    rig.map(0, Source::any(Control::Nrpn(300)));
    let [high, low] = nrpn_address(300);
    rig.block(&[high, low, cc(38, 5)]);
    assert_eq!(rig.value(0), UNTOUCHED, "nothing to refine yet");
    rig.block(&[cc(6, 100)]);
    assert_eq!(rig.value(0), stored(fourteen(100 << 7 | 5)));
    // The low byte is spent: a high byte alone is coarse again.
    rig.block(&[cc(6, 101)]);
    assert_eq!(rig.value(0), stored(fourteen(101 << 7)));
}

#[test]
fn a_new_address_drops_a_waiting_low_byte() {
    let mut rig = rig(1);
    rig.map(0, Source::any(Control::Nrpn(300)));
    let [high, low] = nrpn_address(300);
    rig.block(&[high, low, cc(38, 5)]);
    rig.block(&[high, low, cc(6, 100)]);
    assert_eq!(rig.value(0), stored(fourteen(100 << 7)));
}

#[test]
fn a_new_address_high_byte_forgets_the_old_low_byte() {
    let mut rig = rig(2);
    rig.map(0, Source::any(Control::Nrpn(300)));
    rig.map(1, Source::any(Control::Nrpn(2 << 7)));
    let [high, low] = nrpn_address(300);
    // 300 is 2 << 7 | 44. A high byte alone selects no number: it does not
    // pair with the low byte of the address before it.
    rig.block(&[high, low, cc(99, 2), cc(6, 100), cc(38, 5)]);
    assert_eq!([rig.value(0), rig.value(1)], [UNTOUCHED, UNTOUCHED]);
    rig.block(&[cc(98, 0), cc(6, 100)]);
    assert_eq!(rig.value(1), stored(fourteen(100 << 7)));
}

#[test]
fn reset_all_controllers_forgets_the_address() {
    let mut rig = rig(1);
    rig.map(0, Source::any(Control::Nrpn(300)));
    let [high, low] = nrpn_address(300);
    rig.block(&[high, low, cc(121, 0), cc(98, 44), cc(6, 100)]);
    assert_eq!(
        rig.value(0),
        UNTOUCHED,
        "the high byte of the address is gone"
    );
}

#[test]
fn an_nrpn_address_selects_its_number_whichever_byte_comes_first() {
    // 300 is 2 << 7 | 44.
    for address in [[cc(99, 2), cc(98, 44)], [cc(98, 44), cc(99, 2)]] {
        let mut rig = rig(1);
        rig.map(0, Source::any(Control::Nrpn(300)));
        rig.block(&[address[0], address[1], cc(6, 100)]);
        assert_eq!(rig.value(0), stored(fourteen(100 << 7)), "{address:?}");
        // The same address again, in the same order, after the first was used.
        rig.block(&[address[0], address[1], cc(6, 101)]);
        assert_eq!(rig.value(0), stored(fourteen(101 << 7)), "{address:?}");
    }
}

/// What `juce::MidiRPNGenerator::generate` sends for an NRPN on channel 1:
/// the address low byte first (98, then 99), the value's high byte (6), and
/// its low byte (38) only for a 14-bit value. A 7-bit value goes in 6 alone.
fn juce_nrpn(number: u16, value: u16, fourteen_bit: bool) -> Vec<ChannelVoice> {
    let mut sent = vec![cc(98, (number & 0x7F) as u8), cc(99, (number >> 7) as u8)];
    if fourteen_bit {
        sent.extend([cc(6, (value >> 7) as u8), cc(38, (value & 0x7F) as u8)]);
    } else {
        sent.push(cc(6, value as u8));
    }
    sent
}

#[test]
fn juces_rpn_generator_reaches_its_nrpn_with_or_without_the_low_byte() {
    let mut rig = rig(2);
    rig.map(0, Source::any(Control::Nrpn(300)));
    rig.map(1, Source::any(Control::Nrpn(5)));
    rig.block(&juce_nrpn(300, 100 << 7 | 5, true));
    assert_eq!(rig.value(0), stored(fourteen(100 << 7 | 5)));
    // The next message names another number while 300's bytes are held.
    rig.block(&juce_nrpn(5, 64, false));
    assert_eq!(rig.value(1), stored(fourteen(64 << 7)));
    assert_eq!(rig.value(0), stored(fourteen(100 << 7 | 5)), "5's alone");
    rig.block(&juce_nrpn(300, 16383, true));
    assert_eq!(rig.value(0), 1.0);
    assert_eq!(rig.value(1), stored(fourteen(64 << 7)), "300's alone");
}

#[test]
fn a_lone_high_byte_never_pairs_with_a_low_byte_from_before_a_data_entry() {
    let mut rig = rig(2);
    rig.map(0, Source::any(Control::Nrpn(300)));
    rig.map(1, Source::any(Control::Nrpn(3 << 7 | 44)));
    let [high, low] = nrpn_address(300);
    rig.block(&[high, low, cc(6, 10)]);
    let before = rig.value(0);
    // 44 belongs to the address that data entry used.
    rig.block(&[cc(99, 3), cc(6, 20), cc(38, 5)]);
    assert_eq!([rig.value(0), rig.value(1)], [before, UNTOUCHED]);

    // A low byte sent first is spent by a data entry too, even one that
    // selected nothing.
    let mut rig = self::rig(1);
    rig.map(0, Source::any(Control::Nrpn(300)));
    rig.block(&[cc(98, 44), cc(6, 10), cc(99, 2), cc(6, 20)]);
    assert_eq!(rig.value(0), UNTOUCHED);
}

#[test]
fn data_increment_and_decrement_use_the_address_and_drive_nothing() {
    let mut rig = rig(1);
    rig.map(0, Source::any(Control::Nrpn(300)));
    let [high, low] = nrpn_address(300);
    rig.block(&[high, low, cc(6, 10)]);
    let before = rig.value(0);
    // Learn does not step a parameter number's value.
    rig.block(&[cc(96, 1), cc(97, 1)]);
    assert_eq!(rig.value(0), before);

    // A low byte the increment used does not pair with a high byte after it.
    let mut rig = self::rig(1);
    rig.map(0, Source::any(Control::Nrpn(300)));
    rig.block(&[cc(98, 44), cc(96, 1), cc(99, 2), cc(6, 20)]);
    assert_eq!(rig.value(0), UNTOUCHED);
}
