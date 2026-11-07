//! Where a binding listens.

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

use crate::field::Field;

/// The kind of message a binding listens to, and its number.
///
/// # Examples
///
/// ```
/// use seam_learn::{Control, Source};
///
/// // The kinds a binding can listen to.
/// let knob = Control::Cc(74);
/// let fine_knob = Control::Nrpn(300);
/// let key = Control::Note(60);
/// assert!(Source::any(knob).is_valid() && Source::any(fine_knob).is_valid());
/// assert!(Source::any(key).is_valid());
/// // A controller number is 7 bits.
/// assert!(!Source::any(Control::Cc(200)).is_valid());
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Control {
    /// A control change, 0 to 127, from MIDI 1.0 or MIDI 2.0.
    ///
    /// Controllers 6, 38, 96, 97, 98, 99, 100 and 101 belong to the NRPN and
    /// RPN parser and are never bound. They select a parameter number, enter
    /// its value, or step it.
    ///
    /// A binding reads that controller alone, so a MIDI 1.0 14-bit pair plays
    /// at 7 bits through the byte learn bound: the high byte (below 32) in the
    /// cases [`Learn::poll`](crate::Learn::poll) names. Open: reading both
    /// bytes waits for an exporter that delivers MIDI.
    Cc(u8),
    /// A non-registered (assignable) parameter number, 0 to 16383. In MIDI
    /// 1.0 it is an NRPN, assembled from controllers 99, 98, 6 and 38. In
    /// MIDI 2.0 it is an assignable controller numbered bank × 128 + index.
    Nrpn(u16),
    /// A note, 0 to 127.
    ///
    /// A note-on writes its velocity and a note-off writes nothing. A MIDI 1.0
    /// note-on at velocity 0 is a note-off; a MIDI 2.0 one is a note-on, as
    /// the MIDI 2.0 specification reads it, and writes 0.0.
    Note(u8),
}

impl Control {
    /// The controllers the parser owns, which never act as plain controllers:
    /// an increment's value is a step for the selected number, not a
    /// position, so a binding would jump its target.
    pub(crate) const PARSER_OWNED: [u8; 8] = [6, 38, 96, 97, 98, 99, 100, 101];

    /// The kind's number in a packed word, a dispatch key or a staged
    /// capture.
    pub(crate) fn kind(self) -> u64 {
        match self {
            Control::Cc(_) => 0,
            Control::Nrpn(_) => 1,
            Control::Note(_) => 2,
        }
    }

    /// The control of the kind [`kind`](Self::kind) numbers `kind`, with
    /// `number`; `None` for a number no kind has or one too large for it.
    pub(crate) fn from_kind(kind: u64, number: u16) -> Option<Control> {
        match kind {
            0 => u8::try_from(number).ok().map(Control::Cc),
            1 => Some(Control::Nrpn(number)),
            2 => u8::try_from(number).ok().map(Control::Note),
            _ => None,
        }
    }

    pub(crate) fn number(self) -> u16 {
        match self {
            Control::Cc(number) | Control::Note(number) => u16::from(number),
            Control::Nrpn(number) => number,
        }
    }

    fn in_range(self) -> bool {
        match self {
            Control::Cc(number) | Control::Note(number) => number <= 127,
            Control::Nrpn(number) => number <= 16383,
        }
    }

    pub(crate) fn parser_owned(self) -> bool {
        matches!(self, Control::Cc(number) if Self::PARSER_OWNED.contains(&number))
    }
}

/// Where a binding listens: a control, and the UMP group and channel it
/// must arrive on, each `None` for any. Two controllers on different
/// groups stay apart.
///
/// # Examples
///
/// ```
/// use seam_learn::{Control, Source};
///
/// // Controller 7 on the second channel of the first group only.
/// let source = Source { control: Control::Cc(7), group: Some(0), channel: Some(1) };
/// assert!(source.is_valid());
/// assert_ne!(source, Source::any(Control::Cc(7)));
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Source {
    /// What it listens to.
    pub control: Control,
    /// The UMP group, 0 to 15, or `None` for any. In VST3 the group is the
    /// event bus index.
    pub group: Option<u8>,
    /// The channel, 0 to 15, or `None` for any.
    pub channel: Option<u8>,
}

impl Source {
    /// `control` on any group and any channel.
    ///
    /// # Examples
    ///
    /// ```
    /// use seam_learn::{Control, Source};
    ///
    /// let source = Source::any(Control::Cc(7));
    /// assert_eq!((source.group, source.channel), (None, None));
    /// ```
    pub fn any(control: Control) -> Self {
        Source {
            control,
            group: None,
            channel: None,
        }
    }

    /// Whether every number is in range: a control's number for its kind,
    /// a group and a channel 0 to 15.
    ///
    /// # Examples
    ///
    /// ```
    /// use seam_learn::{Control, Source};
    ///
    /// // A hand-edited number is checked before it becomes a mapping.
    /// let source = Source { control: Control::Cc(7), group: None, channel: Some(16) };
    /// assert!(!source.is_valid());
    /// assert!(Source { channel: Some(15), ..source }.is_valid());
    /// ```
    pub fn is_valid(&self) -> bool {
        self.control.in_range()
            && self.group.is_none_or(|group| group < 16)
            && self.channel.is_none_or(|channel| channel < 16)
    }

    pub(crate) fn key(&self) -> u64 {
        key(self.control, self.group, self.channel)
    }
}

/// The dispatch table's sort key: kind, number, group, channel, with "any"
/// before every group and channel. Kind and number take the high bits, so
/// every key for one control is one run of the table.
pub(crate) fn key(control: Control, group: Option<u8>, channel: Option<u8>) -> u64 {
    // Low bits first. "Any" is 0 and group or channel n is n + 1, which takes
    // five bits.
    const CHANNEL: Field = Field::lowest(5);
    const GROUP: Field = Field::above(CHANNEL, 5);
    const NUMBER: Field = Field::above(GROUP, 14);
    const KIND: Field = Field::above(NUMBER, 2);
    let any_first = |field: Option<u8>| field.map_or(0, |value| u64::from(value) + 1);
    KIND.put(control.kind())
        | NUMBER.put(u64::from(control.number()))
        | GROUP.put(any_first(group))
        | CHANNEL.put(any_first(channel))
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::*;

    fn fields() -> impl Iterator<Item = Option<u8>> + Clone {
        std::iter::once(None).chain((0..16).map(Some))
    }

    /// Edge numbers of every kind, so a shift that lets one field run into
    /// the next shows as a collision.
    fn controls() -> Vec<Control> {
        let sevens = [0, 1, 2, 63, 64, 65, 126, 127];
        let fourteens = [0, 1, 2, 127, 128, 8191, 8192, 16382, 16383];
        let mut controls: Vec<Control> = sevens.iter().map(|&n| Control::Cc(n)).collect();
        controls.extend(fourteens.iter().map(|&n| Control::Nrpn(n)));
        controls.extend(sevens.iter().map(|&n| Control::Note(n)));
        controls
    }

    #[test]
    fn keys_are_unique_and_sort_by_kind_number_then_any_first() {
        let mut seen = HashSet::new();
        let mut sources = Vec::new();
        for control in controls() {
            for group in fields() {
                for channel in fields() {
                    let source = Source {
                        control,
                        group,
                        channel,
                    };
                    assert!(source.is_valid());
                    assert!(seen.insert(source.key()), "{source:?} collides");
                    sources.push(source);
                }
            }
        }
        let order = |source: &Source| {
            let any_first = |field: Option<u8>| field.map_or(0, |value| u16::from(value) + 1);
            (
                source.control,
                any_first(source.group),
                any_first(source.channel),
            )
        };
        for pair in sources.windows(2) {
            assert_eq!(
                pair[0].key().cmp(&pair[1].key()),
                order(&pair[0]).cmp(&order(&pair[1])),
                "{:?} against {:?}",
                pair[0],
                pair[1]
            );
        }
        let mut by_key = sources.clone();
        by_key.sort_by_key(Source::key);
        let mut by_fields = sources;
        by_fields.sort_by_key(order);
        assert_eq!(by_key, by_fields);
    }
}
