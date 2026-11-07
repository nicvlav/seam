//! The words capture shares, and the loom model of its block counts.

use super::*;

#[test]
fn a_staged_capture_round_trips_through_its_word() {
    for control in [
        Control::Cc(127),
        Control::Nrpn(16383),
        Control::Note(0),
        Control::Nrpn(0),
    ] {
        for (group, channel) in [(0, 0), (15, 15), (3, 9)] {
            for resolution in [Resolution::Fine, Resolution::Coarse, Resolution::Waiting] {
                let staged = Staged {
                    generation: u32::MAX,
                    source: Source {
                        control,
                        group: Some(group),
                        channel: Some(channel),
                    },
                    resolution,
                };
                assert_eq!(Staged::unpack(staged.pack()), Some(staged));
            }
        }
    }
    assert_eq!(Staged::unpack(0), None, "generation 0 is empty");
}

#[test]
fn an_arm_round_trips_through_its_word_and_disarmed_is_zero() {
    for generation in [1, 2, u32::MAX] {
        for takes in [
            Takes::CONTROLLER,
            Takes::NOTE,
            Takes::CONTROLLER | Takes::NOTE,
        ] {
            let arm = ArmWord { generation, takes };
            assert_eq!(ArmWord::unpack(arm.pack()), arm);
        }
    }
    assert_eq!(ArmWord::DISARMED.pack(), 0, "a fresh slot is disarmed");
    assert_eq!(ArmWord::unpack(0), ArmWord::DISARMED);
}

#[test]
fn takes_debugs_as_the_kinds_it_takes() {
    for (takes, shown) in [
        (Takes::CONTROLLER, "Takes(CONTROLLER)"),
        (Takes::NOTE, "Takes(NOTE)"),
        (Takes::CONTROLLER | Takes::NOTE, "Takes(CONTROLLER | NOTE)"),
        (Takes(0), "Takes(none)"),
    ] {
        assert_eq!(format!("{takes:?}"), shown);
    }
}

#[cfg(loom)]
mod loom_models {
    use std::sync::Arc;

    use super::super::{Arm, Policy, Slots, Stager, Takes};
    use crate::parse::{Message, Value};
    use crate::{Control, CustomId, TargetId};

    /// NRPN 300's data entry, with its low byte or without.
    fn data_entry(low: Option<u8>) -> Message {
        Message {
            control: Control::Nrpn(300),
            group: 0,
            channel: 0,
            value: Value::Nrpn { high: 64, low },
        }
    }

    /// The audio thread runs two blocks, an NRPN high byte and then its low
    /// byte, while the UI polls three times. However the polls fall, none
    /// settles the high byte as coarse, and a poll after both blocks learns
    /// the 14-bit NRPN. Counting polls alone fails, and so does settling once
    /// the count of blocks begun has moved: a poll can land after the second
    /// block began but before it offered the low byte. Weakening the ended
    /// count's release or acquire fails too.
    #[test]
    fn loom_a_low_byte_in_the_next_block_is_never_missed() {
        loom::model(|| {
            let slots = Arc::new(Slots::default());
            let mut arm = Arm::new(Arc::clone(&slots));
            arm.arm(TargetId::Custom(CustomId(0)), Takes::CONTROLLER);
            let policy = Policy {
                channel_mode: true,
                ccs: Vec::new(),
                nrpns: Vec::new(),
            };
            let mut stager = Stager::new(slots, policy);
            let audio = loom::thread::spawn(move || {
                for message in [data_entry(None), data_entry(Some(5))] {
                    stager.block(|offers| offers.offer(&message));
                }
            });
            let mut learned = None;
            for _ in 0..3 {
                if learned.is_none() {
                    learned = arm.capture();
                }
            }
            audio.join().expect("the audio thread");
            let (_, mapping) = learned
                .or_else(|| arm.capture())
                .expect("a poll after both blocks learns it");
            assert!(!mapping.coarse, "the low byte came in the next block");
        });
    }
}
