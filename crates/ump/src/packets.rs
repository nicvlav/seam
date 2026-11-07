//! Packet framing: how many words a Universal MIDI Packet takes, and
//! splitting a stream of words into packets.

use std::fmt;
use std::iter::FusedIterator;

use crate::Event;

/// The number of 32-bit words in a packet of `message_type`, from the UMP
/// 1.1 table: 1 to 4.
///
/// The reserved types have sizes too, so a reader can skip a packet it does
/// not know. `None` for a value above 15, which is no message type.
///
/// # Examples
///
/// ```
/// use seam_ump::packet_words;
///
/// assert_eq!(packet_words(0x2), Some(1)); // MIDI 1.0 channel voice
/// assert_eq!(packet_words(0x4), Some(2)); // MIDI 2.0 channel voice
/// assert_eq!(packet_words(0x5), Some(4)); // SysEx8
/// assert_eq!(packet_words(0x10), None);
/// ```
pub const fn packet_words(message_type: u8) -> Option<usize> {
    match message_type {
        0x0..=0x2 | 0x6 | 0x7 => Some(1),
        0x3 | 0x4 | 0x8..=0xA => Some(2),
        0xB | 0xC => Some(3),
        0x5 | 0xD..=0xF => Some(4),
        _ => None,
    }
}

/// A stream ended inside a packet: its first word promised more words than
/// the stream has left.
///
/// # Examples
///
/// ```
/// use seam_ump::Packets;
///
/// // A MIDI 2.0 note-on is two words; the stream has only its first.
/// let mut packets = Packets::new(&[0x4090_3C00], 0);
/// let cut = packets.next().expect("one result").expect_err("the packet is cut");
/// assert_eq!((cut.message_type, cut.needed, cut.available), (4, 2, 1));
/// assert_eq!(cut.to_string(), "a type 0x4 packet takes 2 words and 1 were left");
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct Truncated {
    /// The packet's message type.
    pub message_type: u8,
    /// The words that type takes.
    pub needed: usize,
    /// The words that were left.
    pub available: usize,
}

impl fmt::Display for Truncated {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "a type {:#X} packet takes {} words and {} were left",
            self.message_type, self.needed, self.available
        )
    }
}

impl std::error::Error for Truncated {}

/// A stream of 32-bit words split into packets, as [`Event`]s.
///
/// Every packet becomes an event at the offset given to
/// [`new`](Self::new), with the words it has and zero for the rest. A
/// packet the stream's end cuts short gives a [`Truncated`], the last item.
///
/// # Examples
///
/// ```
/// use seam_ump::Packets;
///
/// // A MIDI 1.0 note-on, then a MIDI 2.0 note-on cut after its first word.
/// let stream = [0x2090_3C7F, 0x4090_3C00];
/// let mut packets = Packets::new(&stream, 32);
/// assert_eq!(packets.next().expect("a packet").expect("whole").words[0], 0x2090_3C7F);
/// let cut = packets.next().expect("a result").expect_err("cut");
/// assert_eq!((cut.needed, cut.available), (2, 1));
/// assert_eq!(packets.next(), None);
/// ```
#[derive(Debug, Clone)]
pub struct Packets<'a> {
    words: &'a [u32],
    offset: u32,
}

impl<'a> Packets<'a> {
    /// Splits `words`, giving every event `offset`.
    pub fn new(words: &'a [u32], offset: u32) -> Self {
        Packets { words, offset }
    }
}

impl Iterator for Packets<'_> {
    type Item = Result<Event, Truncated>;

    #[cfg_attr(seam_rtsan, sanitize(realtime = "nonblocking"))]
    fn next(&mut self) -> Option<Self::Item> {
        let first = *self.words.first()?;
        let message_type = (first >> 28) as u8;
        // A four-bit type is always in the table.
        let needed = packet_words(message_type).unwrap_or(1);
        let Some((packet, rest)) = self.words.split_at_checked(needed) else {
            let available = self.words.len();
            self.words = &[];
            return Some(Err(Truncated {
                message_type,
                needed,
                available,
            }));
        };
        let mut words = [0; 4];
        for (slot, word) in words.iter_mut().zip(packet) {
            *slot = *word;
        }
        self.words = rest;
        Some(Ok(Event {
            offset: self.offset,
            words,
        }))
    }
}

// An empty stream stays empty, so `None` repeats.
impl FusedIterator for Packets<'_> {}
