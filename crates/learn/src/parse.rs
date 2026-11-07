//! From packets to learnable messages.
//!
//! The parser runs over every packet of every block, mapped or not, so an
//! NRPN address sent while nothing was mapped is still current when a
//! mapping arrives.

// Audio-path code: a panic here ends a host, so indexing, unwrap, expect,
// panic and unchecked arithmetic are refused, and a site that is provably
// safe says why in an `#[expect]`.
#![cfg_attr(
    not(test),
    deny(
        clippy::indexing_slicing,
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::arithmetic_side_effects
    )
)]

use seam_ump::{ChannelVoice, Event, Voice};

use crate::scale;
use crate::source::Control;

/// A learnable message.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Message {
    pub(crate) control: Control,
    pub(crate) group: u8,
    pub(crate) channel: u8,
    pub(crate) value: Value,
}

/// A message's value at the width it arrived.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum Value {
    /// A MIDI 1.0 controller value or velocity.
    Seven(u8),
    /// A MIDI 1.0 NRPN's data entry: the high byte, and the low byte once
    /// one followed it.
    Nrpn { high: u8, low: Option<u8> },
    /// A MIDI 2.0 velocity.
    Sixteen(u16),
    /// A MIDI 2.0 controller value.
    ThirtyTwo(u32),
}

impl Value {
    /// The value as 0..1 for a binding, [`coarse`](crate::Mapping::coarse) or
    /// not, or `None` when the binding ignores this message.
    pub(crate) fn normalized(self, coarse: bool) -> Option<f64> {
        Some(match self {
            Value::Seven(value) => scale::seven(value),
            Value::Nrpn { high, low: None } if coarse => scale::seven(high),
            Value::Nrpn { low: Some(_), .. } if coarse => return None,
            Value::Nrpn { high, low } => {
                scale::fourteen(u16::from(high) << 7 | u16::from(low.unwrap_or(0)))
            }
            Value::Sixteen(value) => scale::sixteen(value),
            Value::ThirtyTwo(value) => scale::thirty_two(value),
        })
    }
}

/// Which parameter number data entry is for, as MIDI 1.0 selects it.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
enum Selected {
    #[default]
    None,
    Nrpn,
    Rpn,
}

/// One group and channel's MIDI 1.0 parameter-number state.
#[derive(Debug, Clone, Copy, Default)]
struct Channel {
    /// Controllers 99 and 98.
    nrpn: [Option<u8>; 2],
    /// The address byte, 99 or 98, sent since the last data entry, RPN
    /// address or completed NRPN address: the other one completes the
    /// address with it.
    unpaired: Option<u8>,
    selected: Selected,
    /// The data entry high byte (controller 6) since the address last
    /// changed.
    data_high: Option<u8>,
    /// A data entry low byte (controller 38) that arrived before its high
    /// byte, which MIDI 1.0 allows; the high byte completes it.
    data_low: Option<u8>,
}

/// MIDI 1.0's Reset All Controllers. Its recommended response (RP-015)
/// sets the parameter number to null; it still passes on as a controller.
const RESET_ALL_CONTROLLERS: u8 = 121;

/// The null parameter number's two bytes: it selects nothing.
const NULL_NUMBER: u8 = 127;

impl Channel {
    /// Deselects the parameter number and forgets its bytes, so data entry
    /// does nothing until a whole address arrives again.
    fn reset(&mut self) {
        *self = Channel::default();
    }

    /// The address changed, so data entry starts over.
    fn address_changed(&mut self, selected: Selected) {
        self.selected = selected;
        self.data_high = None;
        self.data_low = None;
    }

    fn nrpn_number(&self) -> Option<u16> {
        match (self.selected, self.nrpn) {
            (Selected::Nrpn, [Some(high), Some(low)]) => {
                Some(u16::from(high) << 7 | u16::from(low))
            }
            _ => None,
        }
    }

    /// An NRPN address byte, 99 or 98.
    ///
    /// MIDI 1.0 sends the two in either order (JUCE's generator sends 98
    /// first), so a byte completes the address with the other one when that
    /// one is still unpaired. A 99 with nothing to pair with starts a new
    /// address and forgets the low byte held. That byte belonged to an
    /// address already used, and pairing with it would select a number
    /// nobody sent. A 98 with nothing to pair with changes the low byte under
    /// the 99 held. The pair 127 and 127 is the null number, which selects
    /// nothing.
    fn address_byte(&mut self, controller: u8, value: u8) {
        let pairs = self.unpaired.is_some_and(|waiting| waiting != controller);
        if controller == 99 {
            if !pairs {
                self.nrpn[1] = None;
            }
            self.nrpn[0] = Some(value);
        } else {
            self.nrpn[1] = Some(value);
        }
        self.unpaired = (!pairs).then_some(controller);
        let null = self.nrpn == [Some(NULL_NUMBER), Some(NULL_NUMBER)];
        self.address_changed(if null { Selected::None } else { Selected::Nrpn });
    }

    /// A MIDI 1.0 control change to a parser-owned controller.
    fn owned(&mut self, controller: u8, value: u8) -> Option<(Control, Value)> {
        if let 99 | 98 = controller {
            self.address_byte(controller, value);
            return None;
        }
        // Anything else uses or replaces the address, so an address byte
        // after it starts a new one.
        self.unpaired = None;
        match controller {
            // An RPN address selects RPN, whose data entry never dispatches
            // as the last NRPN's.
            101 | 100 => {
                self.address_changed(Selected::Rpn);
                None
            }
            // A high byte dispatches at once, with the low byte that came
            // before it if one did, and otherwise with none.
            6 => {
                let number = self.nrpn_number();
                let low = self.data_low.take();
                self.data_high = number.map(|_| value);
                Some((Control::Nrpn(number?), Value::Nrpn { high: value, low }))
            }
            // A low byte refines the high byte already sent. With none sent
            // it waits for one; with no address it is dropped.
            38 => {
                let number = self.nrpn_number()?;
                let Some(high) = self.data_high else {
                    self.data_low = Some(value);
                    return None;
                };
                Some((
                    Control::Nrpn(number),
                    Value::Nrpn {
                        high,
                        low: Some(value),
                    },
                ))
            }
            // Data increment and decrement step the selected number's value
            // from what the receiver holds, which learn does not keep, so
            // they drive nothing.
            _ => None,
        }
    }
}

/// The parser: a fixed state per group and channel, made off the audio
/// thread and never grown.
#[derive(Debug)]
pub(crate) struct Parser {
    channels: Box<[[Channel; 16]; 16]>,
}

impl Parser {
    pub(crate) fn new() -> Self {
        Parser {
            channels: Box::new([[Channel::default(); 16]; 16]),
        }
    }

    /// The learnable message in `event`, if it carries one.
    pub(crate) fn parse(&mut self, event: &Event) -> Option<Message> {
        let ChannelVoice {
            group,
            channel,
            voice,
        } = event.channel_voice()?;
        let state = self
            .channels
            .get_mut(usize::from(group))?
            .get_mut(usize::from(channel))?;
        let (control, value) = match voice {
            Voice::Midi1ControlChange { controller, value } => {
                if Control::PARSER_OWNED.contains(&controller) {
                    state.owned(controller, value)?
                } else {
                    if controller == RESET_ALL_CONTROLLERS {
                        state.reset();
                    }
                    (Control::Cc(controller), Value::Seven(value))
                }
            }
            // A MIDI 1.0 note-on at velocity 0 is a note-off, and a note-off
            // writes nothing.
            Voice::Midi1NoteOn { note, velocity } if velocity > 0 => {
                (Control::Note(note), Value::Seven(velocity))
            }
            // A MIDI 2.0 note-on at velocity 0 is a note-on, as its
            // specification says, so it writes 0.0.
            Voice::Midi2NoteOn { note, velocity } => {
                (Control::Note(note), Value::Sixteen(velocity))
            }
            // MIDI 2.0 carries parameter numbers and their relative changes
            // as messages of their own, so the controllers MIDI 1.0 selects,
            // enters and steps them with mean nothing here.
            Voice::Midi2ControlChange { controller, value }
                if !Control::PARSER_OWNED.contains(&controller) =>
            {
                (Control::Cc(controller), Value::ThirtyTwo(value))
            }
            Voice::Midi2AssignableController { bank, index, value } => (
                Control::Nrpn(u16::from(bank) << 7 | u16::from(index)),
                Value::ThirtyTwo(value),
            ),
            _ => return None,
        };
        Some(Message {
            control,
            group,
            channel,
            value,
        })
    }
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;
    use proptest::test_runner::FileFailurePersistence;

    use super::*;

    /// What the last of `history`, one channel's control changes in order,
    /// means: MIDI 1.0's reading, except where Seam departs from it on
    /// purpose.
    ///
    /// The reading goes over the whole history again for each message instead
    /// of keeping state, so it shares no structure with the parser.
    ///
    /// MIDI 1.0: controllers 99 and 98 each latch one byte of the NRPN
    /// number, in either order, and 101 and 100 the RPN's. Data entry is for
    /// the number the newest of the four selected, and 127 and 127 select
    /// nothing. Reset All Controllers (121) sets the number to null, as RP-015
    /// recommends. A data entry high byte (6) sets the low byte to zero, and a
    /// low byte (38) refines the high byte held.
    ///
    /// Seam's deviations, each the parser's documented choice:
    ///
    /// 1. A 99 that begins an address forgets the latched 98, which belonged
    ///    to an address already used ([`begins_an_address`]).
    /// 2. A 38 before any 6 since the address waits, and the first 6 carries
    ///    it. MIDI 1.0 would refine the receiver's own high byte, which learn
    ///    does not hold.
    /// 3. Increment and decrement (96, 97) drive nothing, since learn holds no
    ///    value to step.
    /// 4. Reset All Controllers also forgets both latched bytes, so data entry
    ///    after it waits for a whole address. RP-015's null leaves both bytes
    ///    at 127, so a lone 98 after it would select a number.
    fn model(history: &[(u8, u8)]) -> Option<(Control, Value)> {
        let (&(controller, value), earlier) = history.split_last()?;
        if !matches!(controller, 6 | 38 | 96 | 97 | 98 | 99 | 100 | 101) {
            return Some((Control::Cc(controller), Value::Seven(value)));
        }
        // Address messages only select, and deviation 3 drops the steps.
        if !matches!(controller, 6 | 38) {
            return None;
        }
        let earlier = match earlier.iter().rposition(|&(c, _)| c == 121) {
            Some(reset) => &earlier[reset + 1..],
            None => earlier,
        };
        let newest = |controller: u8, within: &[(u8, u8)]| {
            within
                .iter()
                .rev()
                .find(|&&(c, _)| c == controller)
                .map(|&(_, v)| v)
        };
        let address = earlier.iter().rposition(|&(c, _)| matches!(c, 98..=101))?;
        if !matches!(earlier[address].0, 98 | 99) {
            return None;
        }
        let high = newest(99, earlier)?;
        let since_high = &earlier[earlier.iter().rposition(|&(c, _)| c == 99)? + 1..];
        let forgotten = begins_an_address(earlier) && newest(98, since_high).is_none();
        let low = newest(98, earlier).filter(|_| !forgotten)?;
        if (high, low) == (127, 127) {
            return None;
        }
        let number = u16::from(high) << 7 | u16::from(low);
        let since = &earlier[address + 1..];
        if controller == 6 {
            // Zero, as MIDI 1.0 says, unless deviation 2 holds a 38 for it.
            let first_high = since.iter().all(|&(c, _)| c != 6);
            let low = newest(38, since).filter(|_| first_high);
            return Some((Control::Nrpn(number), Value::Nrpn { high: value, low }));
        }
        // A 38 with no 6 since the address waits for one (deviation 2).
        let high = newest(6, since)?;
        Some((
            Control::Nrpn(number),
            Value::Nrpn {
                high,
                low: Some(value),
            },
        ))
    }

    /// Whether the newest 99 in `history` begins an address, rather than
    /// completing one a 98 began.
    ///
    /// Address bytes pair off two by two from the start of a run of them, and
    /// the first of a pair begins an address. A run ends at any other
    /// parameter-number message and at a byte that repeats the one before it,
    /// so within a run the bytes alternate.
    fn begins_an_address(history: &[(u8, u8)]) -> bool {
        let Some(at) = history.iter().rposition(|&(c, _)| c == 99) else {
            return true;
        };
        let mut before_it = 0;
        let mut later = 99;
        for &(c, _) in history[..at].iter().rev() {
            match c {
                98 | 99 if c != later => {
                    before_it += 1;
                    later = c;
                }
                98 | 99 | 6 | 38 | 96 | 97 | 100 | 101 => break,
                // Other controllers leave the address alone.
                _ => {}
            }
        }
        before_it % 2 == 0
    }

    fn control_change() -> impl Strategy<Value = (u8, u8, u8, u8)> {
        (
            0_u8..2,
            0_u8..2,
            prop::sample::select(vec![
                6_u8, 38, 98, 99, 6, 38, 98, 99, 98, 99, 96, 97, 100, 101, 121, 7,
            ]),
            // 127 often, so the null number comes up.
            prop::sample::select(vec![0_u8, 1, 5, 64, 127, 127, 127]),
        )
    }

    proptest! {
        #![proptest_config(ProptestConfig {
            failure_persistence: Some(Box::new(FileFailurePersistence::Direct(
                "tests/parse.proptest-regressions",
            ))),
            cases: 1024,
            ..ProptestConfig::default()
        })]

        /// Whatever order the controllers arrive in, on whichever channels,
        /// the parser means what the model says.
        #[test]
        fn the_parser_agrees_with_a_model_of_midi_1_0(
            changes in proptest::collection::vec(control_change(), 0..48),
        ) {
            let mut parser = Parser::new();
            let mut histories = std::collections::HashMap::<(u8, u8), Vec<(u8, u8)>>::new();
            for (group, channel, controller, value) in changes {
                let history = histories.entry((group, channel)).or_default();
                history.push((controller, value));
                let event = ChannelVoice {
                    group,
                    channel,
                    voice: Voice::Midi1ControlChange { controller, value },
                }
                .event(0);
                let expected = model(history).map(|(control, value)| Message {
                    control,
                    group,
                    channel,
                    value,
                });
                prop_assert_eq!(parser.parse(&event), expected, "{:?}", history);
            }
        }
    }
}
