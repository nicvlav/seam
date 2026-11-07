//! Channel voice messages, read from and written to packets.

use crate::Event;

/// MIDI 1.0 channel voice messages, one word.
const MIDI1: u32 = 0x2;
/// MIDI 2.0 channel voice messages, two words.
const MIDI2: u32 = 0x4;

const NOTE_OFF: u32 = 0x8;
const NOTE_ON: u32 = 0x9;
const CONTROL_CHANGE: u32 = 0xB;
const REGISTERED_CONTROLLER: u32 = 0x2;
const ASSIGNABLE_CONTROLLER: u32 = 0x3;

/// A channel voice message Seam reads, with the group and channel it came
/// on.
///
/// # Examples
///
/// ```
/// use seam_ump::{ChannelVoice, Voice};
///
/// // Group 2, channel 5: a MIDI 2.0 note-on at full velocity.
/// let voice = ChannelVoice {
///     group: 2,
///     channel: 5,
///     voice: Voice::Midi2NoteOn { note: 64, velocity: u16::MAX },
/// };
/// assert_eq!(voice.words()[0], 0x4295_4000);
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChannelVoice {
    /// The UMP group, 0 to 15.
    pub group: u8,
    /// The channel, 0 to 15.
    pub channel: u8,
    /// What it says.
    pub voice: Voice,
}

/// The channel voice messages Seam reads. Each carries its values at
/// the width its protocol sends them; nothing here rescales.
///
/// # Examples
///
/// ```
/// use seam_ump::Voice;
///
/// // MIDI 1.0 sends a velocity of 0 as a note-off; MIDI 2.0 does not.
/// fn sounds(voice: Voice) -> bool {
///     match voice {
///         Voice::Midi1NoteOn { velocity, .. } => velocity > 0,
///         Voice::Midi2NoteOn { .. } => true,
///         _ => false,
///     }
/// }
/// assert!(!sounds(Voice::Midi1NoteOn { note: 60, velocity: 0 }));
/// assert!(sounds(Voice::Midi2NoteOn { note: 60, velocity: 0 }));
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Voice {
    /// A MIDI 1.0 note-off.
    Midi1NoteOff {
        /// The note, 0 to 127.
        note: u8,
        /// Its release velocity, 0 to 127.
        velocity: u8,
    },
    /// A MIDI 1.0 note-on, as sent: MIDI 1.0 reads a velocity of 0 as a
    /// note-off, and that is the reader's call.
    Midi1NoteOn {
        /// The note, 0 to 127.
        note: u8,
        /// Its velocity, 0 to 127.
        velocity: u8,
    },
    /// A MIDI 1.0 control change.
    Midi1ControlChange {
        /// The controller, 0 to 127.
        controller: u8,
        /// Its value, 0 to 127.
        value: u8,
    },
    /// A MIDI 2.0 note-off. Its attribute is not read.
    Midi2NoteOff {
        /// The note, 0 to 127.
        note: u8,
        /// Its release velocity, 0 to 65535.
        velocity: u16,
    },
    /// A MIDI 2.0 note-on: a velocity of 0 is a note-on too. Its attribute
    /// is not read.
    Midi2NoteOn {
        /// The note, 0 to 127.
        note: u8,
        /// Its velocity, 0 to 65535.
        velocity: u16,
    },
    /// A MIDI 2.0 control change.
    Midi2ControlChange {
        /// The controller, 0 to 127.
        controller: u8,
        /// Its value, 0 to `u32::MAX`.
        value: u32,
    },
    /// A MIDI 2.0 registered controller: MIDI 1.0's RPN as one message.
    Midi2RegisteredController {
        /// The bank, 0 to 127.
        bank: u8,
        /// The index in the bank, 0 to 127.
        index: u8,
        /// Its value, 0 to `u32::MAX`.
        value: u32,
    },
    /// A MIDI 2.0 assignable controller: MIDI 1.0's NRPN as one message.
    Midi2AssignableController {
        /// The bank, 0 to 127.
        bank: u8,
        /// The index in the bank, 0 to 127.
        index: u8,
        /// Its value, 0 to `u32::MAX`.
        value: u32,
    },
}

impl ChannelVoice {
    /// Reads a packet. `None` for any other message, including the channel
    /// voice messages not listed in [`Voice`].
    ///
    /// # Examples
    ///
    /// ```
    /// use seam_ump::{ChannelVoice, Voice};
    ///
    /// // A MIDI 2.0 registered controller: bank 0, index 2, half of full scale.
    /// let message = ChannelVoice::decode([0x4020_0002, 0x8000_0000, 0, 0]);
    /// assert_eq!(
    ///     message.map(|message| message.voice),
    ///     Some(Voice::Midi2RegisteredController { bank: 0, index: 2, value: 0x8000_0000 })
    /// );
    ///
    /// // A pitch bend (status 0xE) is not one Seam reads.
    /// assert_eq!(ChannelVoice::decode([0x20E0_0000, 0, 0, 0]), None);
    /// ```
    #[cfg_attr(seam_rtsan, sanitize(realtime = "nonblocking"))]
    pub fn decode(words: [u32; 4]) -> Option<Self> {
        let head = words[0];
        let status = (head >> 20) & 0xF;
        let byte3 = ((head >> 8) & 0x7F) as u8;
        let byte4 = (head & 0x7F) as u8;
        let voice = match head >> 28 {
            MIDI1 => match status {
                NOTE_OFF => Voice::Midi1NoteOff {
                    note: byte3,
                    velocity: byte4,
                },
                NOTE_ON => Voice::Midi1NoteOn {
                    note: byte3,
                    velocity: byte4,
                },
                CONTROL_CHANGE => Voice::Midi1ControlChange {
                    controller: byte3,
                    value: byte4,
                },
                _ => return None,
            },
            MIDI2 => {
                let data = words[1];
                match status {
                    NOTE_OFF => Voice::Midi2NoteOff {
                        note: byte3,
                        velocity: (data >> 16) as u16,
                    },
                    NOTE_ON => Voice::Midi2NoteOn {
                        note: byte3,
                        velocity: (data >> 16) as u16,
                    },
                    CONTROL_CHANGE => Voice::Midi2ControlChange {
                        controller: byte3,
                        value: data,
                    },
                    REGISTERED_CONTROLLER => Voice::Midi2RegisteredController {
                        bank: byte3,
                        index: byte4,
                        value: data,
                    },
                    ASSIGNABLE_CONTROLLER => Voice::Midi2AssignableController {
                        bank: byte3,
                        index: byte4,
                        value: data,
                    },
                    _ => return None,
                }
            }
            _ => return None,
        };
        Some(ChannelVoice {
            group: ((head >> 24) & 0xF) as u8,
            channel: ((head >> 16) & 0xF) as u8,
            voice,
        })
    }

    /// The packet, with every field masked to its width and a MIDI 2.0
    /// note's attribute left empty.
    ///
    /// # Examples
    ///
    /// ```
    /// use seam_ump::{ChannelVoice, Voice};
    ///
    /// // A value past its width is masked: 200 becomes 72 in a 7-bit field.
    /// let message = ChannelVoice {
    ///     group: 0,
    ///     channel: 0,
    ///     voice: Voice::Midi1ControlChange { controller: 7, value: 200 },
    /// };
    /// assert_eq!(message.words(), [0x20B0_0748, 0, 0, 0]);
    /// ```
    #[cfg_attr(seam_rtsan, sanitize(realtime = "nonblocking"))]
    pub fn words(&self) -> [u32; 4] {
        let seven = |byte: u8| u32::from(byte & 0x7F);
        let head = |kind: u32, status: u32, byte3: u8, byte4: u8| {
            kind << 28
                | u32::from(self.group & 0xF) << 24
                | status << 20
                | u32::from(self.channel & 0xF) << 16
                | seven(byte3) << 8
                | seven(byte4)
        };
        match self.voice {
            Voice::Midi1NoteOff { note, velocity } => {
                [head(MIDI1, NOTE_OFF, note, velocity), 0, 0, 0]
            }
            Voice::Midi1NoteOn { note, velocity } => {
                [head(MIDI1, NOTE_ON, note, velocity), 0, 0, 0]
            }
            Voice::Midi1ControlChange { controller, value } => {
                [head(MIDI1, CONTROL_CHANGE, controller, value), 0, 0, 0]
            }
            Voice::Midi2NoteOff { note, velocity } => [
                head(MIDI2, NOTE_OFF, note, 0),
                u32::from(velocity) << 16,
                0,
                0,
            ],
            Voice::Midi2NoteOn { note, velocity } => [
                head(MIDI2, NOTE_ON, note, 0),
                u32::from(velocity) << 16,
                0,
                0,
            ],
            Voice::Midi2ControlChange { controller, value } => {
                [head(MIDI2, CONTROL_CHANGE, controller, 0), value, 0, 0]
            }
            Voice::Midi2RegisteredController { bank, index, value } => {
                [head(MIDI2, REGISTERED_CONTROLLER, bank, index), value, 0, 0]
            }
            Voice::Midi2AssignableController { bank, index, value } => {
                [head(MIDI2, ASSIGNABLE_CONTROLLER, bank, index), value, 0, 0]
            }
        }
    }

    /// The packet as an event at `offset`, ready to push on an
    /// [`EventList`](crate::EventList). The [crate overview](crate#examples)
    /// shows one pushed and read back.
    #[cfg_attr(seam_rtsan, sanitize(realtime = "nonblocking"))]
    pub fn event(&self, offset: u32) -> Event {
        Event {
            offset,
            words: self.words(),
        }
    }
}
