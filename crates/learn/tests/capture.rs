//! The learn flow: arming, capture reservations, and when a capture commits.

use std::sync::Arc;

use seam_learn::{
    CcReservation, Config, Control, Learn, LearnAudio, LearnError, Learned, Mapping,
    NrpnReservation, Source, Takes,
};
use seam_params::{Kind, ParamDecl, ParamId, ParamSet, ParamSetBuilder};
use seam_ump::{ChannelVoice, EventList, Voice};

fn set_of(count: usize) -> Arc<ParamSet> {
    let mut builder = ParamSetBuilder::new();
    for index in 0..count {
        let key: &'static str = Box::leak(format!("p{index}").into_boxed_str());
        builder
            .add(ParamDecl::new(
                key,
                key,
                Kind::continuous(0.0, 1.0, 0.0, None),
                0.5,
            ))
            .expect("valid");
    }
    builder.build()
}

struct Rig {
    learn: Learn,
    audio: LearnAudio,
    ids: Vec<ParamId>,
}

fn rig_with(config: Config) -> Rig {
    let set = set_of(3);
    let (learn, audio) = Learn::new(&set, config);
    Rig {
        learn,
        audio,
        ids: set.ids().collect(),
    }
}

fn rig() -> Rig {
    rig_with(Config::default())
}

impl Rig {
    fn block(&mut self, voices: &[ChannelVoice]) {
        let mut events = EventList::with_capacity(voices.len());
        for (offset, voice) in voices.iter().enumerate() {
            events.push(voice.event(offset as u32));
        }
        self.audio.process(&events);
    }
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

fn exact(control: Control, group: u8, channel: u8) -> Source {
    Source {
        control,
        group: Some(group),
        channel: Some(channel),
    }
}

fn learned(target: ParamId, source: Source, coarse: bool) -> Option<Learned> {
    let mut mapping = Mapping::new(source);
    mapping.coarse = coarse;
    Some(Learned::Mapped {
        target: target.into(),
        mapping,
    })
}

/// An NRPN's address on group 0, channel 0.
fn nrpn_address(number: u16) -> [ChannelVoice; 2] {
    [cc(99, (number >> 7) as u8), cc(98, (number & 0x7F) as u8)]
}

#[test]
fn arming_captures_the_next_message_and_the_poll_commits_it() {
    // The mapping is on the group and channel it arrived on, through the
    // identity curve; the arm is over.
    let mut rig = rig();
    let target = rig.ids[1];
    assert_eq!(rig.learn.poll(), None, "nothing armed");
    rig.learn.arm(target, Takes::CONTROLLER).expect("known");
    assert_eq!(rig.learn.armed(), Some((target.into(), Takes::CONTROLLER)));
    assert_eq!(rig.learn.poll(), None, "nothing staged");
    rig.block(&[on(
        3,
        9,
        Voice::Midi1ControlChange {
            controller: 7,
            value: 64,
        },
    )]);
    let source = exact(Control::Cc(7), 3, 9);
    assert_eq!(rig.learn.poll(), learned(target, source, false));
    assert_eq!(rig.learn.mapping(target), Some(&Mapping::new(source)));
    assert_eq!(rig.learn.armed(), None);
    rig.block(&[cc(8, 1)]);
    assert_eq!(rig.learn.poll(), None, "disarmed");
    assert_eq!(rig.learn.mapping(target).map(|m| m.source), Some(source));
}

#[test]
fn several_captures_before_a_poll_commit_only_the_newest_legal_one() {
    // A refused message later in a tick does not hide an earlier legal one,
    // across blocks too.
    let mut config = Config::default();
    config.reserved_ccs = vec![CcReservation::new(None, None, 20, 29).expect("valid")];
    let mut rig = rig_with(config);
    let target = rig.ids[0];
    rig.learn.arm(target, Takes::CONTROLLER).expect("known");
    rig.block(&[cc(1, 1), cc(2, 2)]);
    rig.block(&[
        cc(3, 3),
        cc(21, 0),
        cc(123, 0),
        on(
            0,
            0,
            Voice::Midi1NoteOn {
                note: 60,
                velocity: 100,
            },
        ),
    ]);
    assert_eq!(
        rig.learn.poll(),
        learned(target, exact(Control::Cc(3), 0, 0), false)
    );
}

#[test]
fn a_refused_message_leaves_the_target_armed() {
    // A reserved controller, a channel mode message, a reserved
    // NRPN and a kind the arm does not take are never captured.
    let mut config = Config::default();
    config.reserved_ccs = vec![CcReservation::new(Some(0), Some(0), 20, 29).expect("valid")];
    config.reserved_nrpns = vec![NrpnReservation::new(None, None, 500, 600).expect("valid")];
    let mut rig = rig_with(config);
    let target = rig.ids[0];
    rig.learn.arm(target, Takes::NOTE).expect("known");
    rig.block(&[cc(5, 5)]);
    assert_eq!(rig.learn.poll(), None, "a note arm takes no controller");
    rig.learn.arm(target, Takes::CONTROLLER).expect("known");
    let [high, low] = nrpn_address(550);
    rig.block(&[
        cc(25, 1),
        cc(120, 0),
        cc(127, 0),
        high,
        low,
        cc(6, 9),
        cc(38, 1),
    ]);
    rig.block(&[on(
        0,
        0,
        Voice::Midi1NoteOn {
            note: 60,
            velocity: 100,
        },
    )]);
    assert_eq!(rig.learn.poll(), None);
    assert_eq!(rig.learn.armed(), Some((target.into(), Takes::CONTROLLER)));

    // The same controller on another channel is free; so is the mode
    // range when the plugin turns its reservation off.
    rig.block(&[on(
        0,
        1,
        Voice::Midi1ControlChange {
            controller: 25,
            value: 1,
        },
    )]);
    assert_eq!(
        rig.learn.poll(),
        learned(target, exact(Control::Cc(25), 0, 1), false)
    );
    let mut config = Config::default();
    config.reserve_channel_mode = false;
    let mut rig = rig_with(config);
    rig.learn
        .arm(rig.ids[0], Takes::CONTROLLER | Takes::NOTE)
        .expect("known");
    rig.block(&[cc(123, 0)]);
    assert_eq!(
        rig.learn.poll(),
        learned(rig.ids[0], exact(Control::Cc(123), 0, 0), false)
    );
}

#[test]
fn a_reserved_controller_can_still_be_mapped_by_hand_and_dispatches() {
    // Naming a reserved controller is intent, so the mapping is allowed.
    let set = set_of(1);
    let mut config = Config::default();
    config.reserved_ccs = vec![CcReservation::new(None, None, 20, 29).expect("valid")];
    let (mut learn, mut audio) = Learn::new(&set, config);
    let target = set.ids().next().expect("one");
    learn
        .map(target, Mapping::new(Source::any(Control::Cc(120))))
        .expect("allowed by hand");
    let mut events = EventList::with_capacity(1);
    events.push(cc(120, 127).event(0));
    audio.process(&events);
    assert_eq!(set.value(target), 1.0);
}

#[test]
fn a_message_staged_for_one_arm_never_commits_to_the_next() {
    let mut rig = rig();
    rig.learn.arm(rig.ids[0], Takes::CONTROLLER).expect("known");
    rig.block(&[cc(1, 1)]);
    rig.learn.arm(rig.ids[1], Takes::CONTROLLER).expect("known");
    assert_eq!(rig.learn.poll(), None, "staged before this arm");
    assert!(rig.learn.is_empty());
    rig.block(&[cc(2, 2)]);
    assert_eq!(
        rig.learn.poll(),
        learned(rig.ids[1], exact(Control::Cc(2), 0, 0), false)
    );
    assert_eq!(rig.learn.len(), 1);

    // An NRPN high byte waiting for one arm does not settle into the next.
    rig.learn.arm(rig.ids[0], Takes::CONTROLLER).expect("known");
    let [high, low] = nrpn_address(300);
    rig.block(&[high, low, cc(6, 64)]);
    rig.learn.arm(rig.ids[2], Takes::CONTROLLER).expect("known");
    rig.block(&[]);
    rig.block(&[]);
    assert_eq!(rig.learn.poll(), None, "nothing was sent for this arm");
}

#[test]
fn cancelling_leaves_the_mappings_alone_and_drops_what_was_staged() {
    let mut rig = rig();
    let source = Source::any(Control::Cc(9));
    rig.learn
        .map(rig.ids[0], Mapping::new(source))
        .expect("legal");
    assert!(!rig.learn.cancel(), "nothing armed");
    rig.learn.arm(rig.ids[0], Takes::CONTROLLER).expect("known");
    rig.block(&[cc(1, 1)]);
    assert!(rig.learn.cancel());
    assert_eq!(rig.learn.poll(), None);
    assert_eq!(rig.learn.mapping(rig.ids[0]), Some(&Mapping::new(source)));
}

#[test]
fn editing_the_armed_targets_mapping_cancels_the_arm() {
    // A later message must not overwrite the user's change.
    for unmap in [false, true] {
        let mut rig = rig();
        let target = rig.ids[0];
        rig.learn
            .map(target, Mapping::new(Source::any(Control::Cc(9))))
            .expect("legal");
        rig.learn.arm(target, Takes::CONTROLLER).expect("known");
        let change = Mapping::new(Source::any(Control::Cc(10)));
        if unmap {
            rig.learn.unmap(target);
        } else {
            rig.learn.map(target, change).expect("legal");
        }
        assert_eq!(rig.learn.armed(), None);
        rig.block(&[cc(1, 1)]);
        assert_eq!(rig.learn.poll(), None);
        assert_eq!(rig.learn.mapping(target), (!unmap).then_some(&change));

        // Changing another target's mapping leaves the arm.
        rig.learn.arm(target, Takes::CONTROLLER).expect("known");
        rig.learn.map(rig.ids[2], change).expect("legal");
        assert_eq!(rig.learn.armed(), Some((target.into(), Takes::CONTROLLER)));
    }
}

#[test]
fn arming_an_unknown_target_is_refused_and_a_full_table_ends_the_arm() {
    let mut config = Config::default();
    config.max_mappings = 1;
    let mut rig = rig_with(config);
    let stranger = set_of(100).ids().nth(99).expect("a hundred");
    assert_eq!(
        rig.learn.arm(stranger, Takes::CONTROLLER),
        Err(LearnError::UnknownTarget(stranger.into()))
    );
    assert_eq!(rig.learn.armed(), None);

    rig.learn
        .map(rig.ids[0], Mapping::new(Source::any(Control::Cc(9))))
        .expect("legal");
    rig.learn.arm(rig.ids[1], Takes::CONTROLLER).expect("known");
    rig.block(&[cc(1, 1)]);
    assert_eq!(
        rig.learn.poll(),
        Some(Learned::Refused {
            target: rig.ids[1].into(),
            error: LearnError::Full
        })
    );
    assert_eq!((rig.learn.len(), rig.learn.armed()), (1, None));
    // Re-pointing the mapped target is free.
    rig.learn.arm(rig.ids[0], Takes::CONTROLLER).expect("known");
    rig.block(&[cc(2, 1)]);
    assert!(matches!(rig.learn.poll(), Some(Learned::Mapped { .. })));
}

#[test]
fn an_nrpn_high_byte_waits_for_its_low_byte_through_polls_with_no_block_between() {
    // Over DIN, CC 6 and CC 38 often straddle a block, and a poll between
    // them must not learn a 14-bit sender as 7-bit. At blocks longer than a
    // UI tick, two polls can run before the block that brings the low byte:
    // 1,024 frames at 44.1 kHz is 23 ms, a 60 Hz tick 16.7 ms.
    for polls in 1..=2 {
        let mut rig = rig();
        let target = rig.ids[0];
        let source = exact(Control::Nrpn(300), 0, 0);
        rig.learn.arm(target, Takes::CONTROLLER).expect("known");
        let [high, low] = nrpn_address(300);
        rig.block(&[cc(1, 1), high, low, cc(6, 64)]);
        assert_eq!(
            rig.learn.poll(),
            None,
            "the high byte waits, and hides cc 1"
        );
        after_more_polls(&mut rig, polls);
        rig.block(&[cc(38, 5)]);
        assert_eq!(
            rig.learn.poll(),
            learned(target, source, false),
            "14-bit after {polls} polls"
        );
    }
}

/// Polls until `polls` polls in all have run since the last block, each
/// finding nothing settled, since no block ran between them.
fn after_more_polls(rig: &mut Rig, polls: usize) {
    for _ in 1..polls {
        assert_eq!(rig.learn.poll(), None, "no block ran since the last poll");
    }
}

#[test]
fn an_nrpn_high_byte_alone_learns_as_coarse_after_a_tick_or_a_second_high_byte() {
    // The sender sends no low byte. A low byte two blocks late,
    // as at small buffers, still makes it 14-bit if it comes within a tick.
    let mut rig = rig();
    let target = rig.ids[0];
    let source = exact(Control::Nrpn(300), 0, 0);
    let [high, low] = nrpn_address(300);

    rig.learn.arm(target, Takes::CONTROLLER).expect("known");
    rig.block(&[high, low, cc(6, 64)]);
    rig.block(&[]);
    rig.block(&[]);
    assert_eq!(rig.learn.poll(), None, "blocks alone settle nothing");
    rig.block(&[]);
    assert_eq!(
        rig.learn.poll(),
        learned(target, source, true),
        "a tick passed, and a whole block ran between the polls with nothing new"
    );

    rig.learn.arm(target, Takes::CONTROLLER).expect("known");
    rig.block(&[high, low, cc(6, 64)]);
    assert_eq!(rig.learn.poll(), None);
    rig.block(&[]);
    rig.block(&[cc(38, 1)]);
    assert_eq!(
        rig.learn.poll(),
        learned(target, source, false),
        "a low byte two blocks later"
    );

    rig.learn.arm(target, Takes::CONTROLLER).expect("known");
    rig.block(&[high, low, cc(6, 64), cc(6, 65)]);
    assert_eq!(
        rig.learn.poll(),
        learned(target, source, true),
        "a second high byte"
    );

    // Once settled, its later high bytes stay settled as what they were.
    rig.learn.arm(target, Takes::CONTROLLER).expect("known");
    rig.block(&[high, low, cc(6, 64), cc(38, 1)]);
    rig.block(&[cc(6, 70)]);
    assert_eq!(rig.learn.poll(), learned(target, source, false));
}

#[test]
fn a_midi_2_assignable_controller_learns_as_an_nrpn() {
    // The number is bank × 128 + index, always at full width.
    let mut rig = rig();
    rig.learn.arm(rig.ids[0], Takes::CONTROLLER).expect("known");
    rig.block(&[on(
        2,
        4,
        Voice::Midi2AssignableController {
            bank: 3,
            index: 7,
            value: u32::MAX,
        },
    )]);
    assert_eq!(
        rig.learn.poll(),
        learned(rig.ids[0], exact(Control::Nrpn(3 * 128 + 7), 2, 4), false)
    );
}

#[test]
fn a_note_arm_learns_a_note_on_and_never_a_note_off() {
    // A MIDI 1.0 note-on at velocity 0 is a note-off.
    let mut rig = rig();
    rig.learn.arm(rig.ids[0], Takes::NOTE).expect("known");
    rig.block(&[
        on(
            0,
            0,
            Voice::Midi1NoteOn {
                note: 60,
                velocity: 0,
            },
        ),
        on(
            0,
            0,
            Voice::Midi1NoteOff {
                note: 61,
                velocity: 9,
            },
        ),
    ]);
    assert_eq!(rig.learn.poll(), None);
    rig.block(&[on(
        0,
        0,
        Voice::Midi1NoteOn {
            note: 62,
            velocity: 1,
        },
    )]);
    assert_eq!(
        rig.learn.poll(),
        learned(rig.ids[0], exact(Control::Note(62), 0, 0), false)
    );
}

#[test]
fn a_newer_message_clears_a_high_byte_still_waiting() {
    // An NRPN this arm already settled, or a controller, is newer
    // than a high byte waiting for its low byte, which must not later pass
    // for a second high byte and settle over it.
    let mut rig = rig();
    let target = rig.ids[0];
    let [high300, low300] = nrpn_address(300);
    let [high400, low400] = nrpn_address(400);
    rig.learn.arm(target, Takes::CONTROLLER).expect("known");
    rig.block(&[high300, low300, cc(6, 1), cc(38, 1)]);
    rig.block(&[high400, low400, cc(6, 1)]);
    rig.block(&[high300, low300, cc(6, 2)]);
    rig.block(&[high400, low400, cc(6, 3)]);
    assert_eq!(
        rig.learn.poll(),
        None,
        "400's high byte waits, as a first one"
    );

    rig.learn.arm(target, Takes::CONTROLLER).expect("known");
    rig.block(&[high300, low300, cc(6, 64)]);
    rig.block(&[cc(1, 1)]);
    rig.block(&[cc(6, 65)]);
    assert_eq!(
        rig.learn.poll(),
        None,
        "a first high byte again, after cc 1"
    );
}

#[test]
fn a_controller_reservation_on_one_group_leaves_the_others_free() {
    let mut config = Config::default();
    config.reserved_ccs = vec![CcReservation::new(Some(1), None, 20, 29).expect("valid")];
    let mut rig = rig_with(config);
    rig.learn.arm(rig.ids[0], Takes::CONTROLLER).expect("known");
    rig.block(&[on(
        1,
        0,
        Voice::Midi1ControlChange {
            controller: 25,
            value: 1,
        },
    )]);
    assert_eq!(rig.learn.poll(), None);
    rig.block(&[on(
        0,
        0,
        Voice::Midi1ControlChange {
            controller: 25,
            value: 1,
        },
    )]);
    assert_eq!(
        rig.learn.poll(),
        learned(rig.ids[0], exact(Control::Cc(25), 0, 0), false)
    );
}

#[test]
fn dropping_the_audio_side_is_silent_off_the_audio_thread() {
    drop(rig());
}

#[cfg(debug_assertions)]
#[test]
#[should_panic(expected = "dropping LearnAudio must not run on the audio thread")]
fn dropping_the_audio_side_is_refused_on_the_audio_thread() {
    let Rig { audio, .. } = rig();
    let _guard = seam_params::audio_thread::enter();
    drop(audio);
}

#[test]
fn a_14_bit_controller_pair_learns_its_high_byte() {
    // CC 7 with CC 39 below it, in one block or across two.
    for blocks in [
        vec![vec![cc(7, 100), cc(39, 33)]],
        vec![vec![cc(7, 100)], vec![cc(39, 33)]],
        vec![vec![cc(7, 100)], vec![], vec![cc(39, 33), cc(39, 34)]],
    ] {
        let mut rig = rig();
        let target = rig.ids[0];
        rig.learn.arm(target, Takes::CONTROLLER).expect("known");
        for block in &blocks {
            rig.block(block);
        }
        let source = exact(Control::Cc(7), 0, 0);
        assert_eq!(
            rig.learn.poll(),
            learned(target, source, false),
            "{blocks:?}"
        );
    }
}

/// Polls, runs an empty block, and polls again: a waiting capture commits at
/// the second poll, once a whole block ran between the two.
fn after_its_wait(rig: &mut Rig) -> Option<Learned> {
    assert_eq!(rig.learn.poll(), None, "it waits");
    rig.block(&[]);
    rig.learn.poll()
}

#[test]
fn an_arm_between_a_pairs_halves_waits_through_polls_and_learns_the_high_byte_that_follows() {
    // The knob's first move after the arm shows only its low byte. Its next
    // sends the high byte first, in a block after one poll or after two with
    // no block between them.
    for polls in 1..=2 {
        let mut rig = rig();
        let target = rig.ids[0];
        rig.learn.arm(target, Takes::CONTROLLER).expect("known");
        rig.block(&[cc(39, 33)]);
        assert_eq!(rig.learn.poll(), None, "the low byte waits");
        after_more_polls(&mut rig, polls);
        rig.block(&[cc(7, 100), cc(39, 34)]);
        assert_eq!(
            rig.learn.poll(),
            learned(target, exact(Control::Cc(7), 0, 0), false),
            "after {polls} polls"
        );
    }
}

#[test]
fn a_controller_32_to_63_learns_itself_a_tick_later_unless_its_high_byte_came_just_before() {
    let mut rig = rig();
    let target = rig.ids[0];
    let low = |channel| {
        on(
            0,
            channel,
            Voice::Midi1ControlChange {
                controller: 39,
                value: 33,
            },
        )
    };
    let high = |channel| {
        on(
            0,
            channel,
            Voice::Midi1ControlChange {
                controller: 7,
                value: 100,
            },
        )
    };
    let itself = learned(target, exact(Control::Cc(39), 0, 0), false);

    // Alone, it waits for a high byte, then is a controller like any other.
    rig.learn.arm(target, Takes::CONTROLLER).expect("known");
    rig.block(&[low(0)]);
    assert_eq!(after_its_wait(&mut rig), itself);

    // A sender that sends only low bytes, turned slowly: a move between the
    // two polls stages the same capture again, so the second poll learns the
    // low byte. Open: whether such a sender should learn its high byte.
    rig.learn.arm(target, Takes::CONTROLLER).expect("known");
    rig.block(&[low(0)]);
    assert_eq!(rig.learn.poll(), None, "it waits");
    rig.block(&[cc(39, 34)]);
    assert_eq!(rig.learn.poll(), itself);

    // A high byte on another channel, or another controller 32 below, is not
    // its pair.
    rig.learn.arm(target, Takes::CONTROLLER).expect("known");
    rig.block(&[high(1), cc(8, 1), low(0)]);
    assert_eq!(after_its_wait(&mut rig), itself);

    // Nor is its own high byte with another capture between them.
    rig.learn.arm(target, Takes::CONTROLLER).expect("known");
    rig.block(&[high(0), cc(8, 1), low(0)]);
    assert_eq!(after_its_wait(&mut rig), itself);

    // A high byte captured under an earlier arm does not count for the next.
    rig.learn.arm(target, Takes::CONTROLLER).expect("known");
    rig.block(&[high(0)]);
    rig.learn.arm(target, Takes::CONTROLLER).expect("known");
    rig.block(&[low(0)]);
    assert_eq!(after_its_wait(&mut rig), itself);

    // Nor does one on another group, on the same channel.
    rig.learn.arm(target, Takes::CONTROLLER).expect("known");
    rig.block(&[
        on(
            1,
            0,
            Voice::Midi1ControlChange {
                controller: 7,
                value: 100,
            },
        ),
        low(0),
    ]);
    assert_eq!(after_its_wait(&mut rig), itself);

    // Low byte first, then the high byte: the high byte is the newest, and
    // commits at once.
    rig.learn.arm(target, Takes::CONTROLLER).expect("known");
    rig.block(&[low(0), high(0)]);
    assert_eq!(
        rig.learn.poll(),
        learned(target, exact(Control::Cc(7), 0, 0), false)
    );
}

#[test]
fn a_capture_held_back_between_a_high_byte_and_its_low_byte_stops_the_pairing() {
    // A nudge of controller 7, then an NRPN high byte, which the poll holds
    // back and never commits, then controller 39 from another knob: the NRPN
    // came between, so 39 learns itself.
    let mut rig = rig();
    let target = rig.ids[0];
    let [high, low] = nrpn_address(300);
    rig.learn.arm(target, Takes::CONTROLLER).expect("known");
    rig.block(&[cc(7, 100)]);
    rig.block(&[high, low, cc(6, 64)]);
    assert_eq!(rig.learn.poll(), None, "the NRPN high byte waits");
    rig.block(&[cc(39, 33)]);
    assert_eq!(
        after_its_wait(&mut rig),
        learned(target, exact(Control::Cc(39), 0, 0), false)
    );
}

#[test]
fn data_increment_and_decrement_are_never_learned() {
    // Their value is a step for the selected parameter number, not a
    // position a target could follow.
    let mut rig = rig();
    let target = rig.ids[0];
    rig.learn.arm(target, Takes::CONTROLLER).expect("known");
    rig.block(&[cc(96, 1), cc(97, 1)]);
    assert_eq!(rig.learn.poll(), None);
    assert_eq!(rig.learn.poll(), None, "nothing was waiting either");
    rig.block(&[cc(74, 1)]);
    assert_eq!(
        rig.learn.poll(),
        learned(target, exact(Control::Cc(74), 0, 0), false)
    );
}
