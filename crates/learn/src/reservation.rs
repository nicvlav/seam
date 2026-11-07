//! What learn never captures or never binds: controller and NRPN numbers a
//! plugin reserves, on a group and a channel or on every one.

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

use std::fmt;
use std::ops::RangeInclusive;

use crate::{Control, Source};

/// Why a reservation was refused: the field that is out of range.
///
/// # Examples
///
/// ```
/// use seam_learn::{CcReservation, ReservationError};
///
/// assert_eq!(CcReservation::new(Some(16), None, 0, 1), Err(ReservationError::Group));
/// assert_eq!(CcReservation::new(None, None, 0, 128), Err(ReservationError::Number));
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum ReservationError {
    /// The group is above 15.
    Group,
    /// The channel is above 15.
    Channel,
    /// A number is above the largest the control allows: 127 for a
    /// controller, 16383 for an NRPN.
    Number,
}

impl fmt::Display for ReservationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ReservationError::Group => write!(f, "a reservation's group must be 0 to 15"),
            ReservationError::Channel => write!(f, "a reservation's channel must be 0 to 15"),
            ReservationError::Number => {
                write!(f, "a reservation's number is above the control's largest")
            }
        }
    }
}

impl std::error::Error for ReservationError {}

/// Checks a reservation's group and channel, `None` meaning every one.
pub(crate) fn check_fields(group: Option<u8>, channel: Option<u8>) -> Result<(), ReservationError> {
    if group.is_some_and(|group| group >= 16) {
        return Err(ReservationError::Group);
    }
    if channel.is_some_and(|channel| channel >= 16) {
        return Err(ReservationError::Channel);
    }
    Ok(())
}

/// Controller numbers learn never captures, a plugin's own controls say.
/// They stay mappable by hand ([`Learn::map`](crate::Learn::map)).
///
/// # Examples
///
/// ```
/// use seam_learn::{CcReservation, Config, Learn, Learned, Takes};
#[doc = include_str!("../doc/setup.rs")]
/// // The plugin's own controllers 20 to 23 are never learned, on any channel.
/// let reserved = CcReservation::new(None, None, 20, 23)?;
/// let mut config = Config::default();
/// config.reserved_ccs.push(reserved);
/// let (mut learn, mut audio) = Learn::new(&set, config);
///
/// learn.arm(gain, Takes::CONTROLLER)?;
/// audio.process(&block(&[(21, 127)])); // controller 21 moves
/// assert_eq!(learn.poll(), None, "refused, and still armed");
/// audio.process(&block(&[(7, 127)])); // controller 7 moves
/// assert!(matches!(learn.poll(), Some(Learned::Mapped { .. })));
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CcReservation {
    group: Option<u8>,
    channel: Option<u8>,
    numbers: RangeInclusive<u8>,
}

impl CcReservation {
    /// Creates a reservation of the controllers from `first` to `last`, either
    /// way round, on `group` and `channel` (each `None` for every one). The
    /// [`CcReservation`] example reserves a range.
    ///
    /// # Errors
    ///
    /// Returns the [`ReservationError`] naming the first field out of range:
    /// a group or channel above 15, or a number above 127.
    pub fn new(
        group: Option<u8>,
        channel: Option<u8>,
        first: u8,
        last: u8,
    ) -> Result<Self, ReservationError> {
        check_fields(group, channel)?;
        let numbers = first.min(last)..=first.max(last);
        if *numbers.end() > 127 {
            return Err(ReservationError::Number);
        }
        Ok(CcReservation {
            group,
            channel,
            numbers,
        })
    }

    /// Returns the group it applies on, or `None` for every group.
    pub fn group(&self) -> Option<u8> {
        self.group
    }

    /// Returns the channel it applies on, or `None` for every channel.
    pub fn channel(&self) -> Option<u8> {
        self.channel
    }

    /// Returns the controllers it reserves, first to last.
    pub fn numbers(&self) -> RangeInclusive<u8> {
        self.numbers.clone()
    }

    pub(crate) fn covers(&self, group: u8, channel: u8, number: u8) -> bool {
        self.group.is_none_or(|reserved| reserved == group)
            && self.channel.is_none_or(|reserved| reserved == channel)
            && self.numbers.contains(&number)
    }
}

/// NRPN numbers a plugin's own protocol owns: never learned and never
/// dispatched.
///
/// A mapping whose source could hear any message a reservation covers is
/// refused, so an any-channel mapping of a number reserved on one channel is
/// refused too.
///
/// # Examples
///
/// ```
/// use seam_learn::{Config, Control, Learn, LearnError, Mapping, NrpnReservation, Source};
#[doc = include_str!("../doc/setup.rs")]
/// // The plugin's own protocol owns NRPNs 0 to 99: they cannot be mapped.
/// let reserved = NrpnReservation::new(None, None, 0, 99)?;
/// let mut config = Config::default();
/// config.reserved_nrpns.push(reserved);
/// let (mut learn, _audio) = Learn::new(&set, config);
///
/// let mine = Mapping::new(Source::any(Control::Nrpn(50)));
/// assert!(matches!(learn.map(gain, mine), Err(LearnError::Blocked(_))));
/// let free = Mapping::new(Source::any(Control::Nrpn(300)));
/// assert!(learn.map(gain, free).is_ok());
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NrpnReservation {
    group: Option<u8>,
    channel: Option<u8>,
    numbers: RangeInclusive<u16>,
}

impl NrpnReservation {
    /// Creates a reservation of the numbers from `first` to `last`, either way
    /// round, on `group` and `channel` (each `None` for every one). The
    /// [`NrpnReservation`] example reserves a range.
    ///
    /// # Errors
    ///
    /// Returns the [`ReservationError`] naming the first field out of range:
    /// a group or channel above 15, or a number above 16383.
    pub fn new(
        group: Option<u8>,
        channel: Option<u8>,
        first: u16,
        last: u16,
    ) -> Result<Self, ReservationError> {
        check_fields(group, channel)?;
        let numbers = first.min(last)..=first.max(last);
        if *numbers.end() > 16383 {
            return Err(ReservationError::Number);
        }
        Ok(NrpnReservation {
            group,
            channel,
            numbers,
        })
    }

    /// Returns the group it applies on, or `None` for every group.
    pub fn group(&self) -> Option<u8> {
        self.group
    }

    /// Returns the channel it applies on, or `None` for every channel.
    pub fn channel(&self) -> Option<u8> {
        self.channel
    }

    /// Returns the numbers it reserves, first to last.
    pub fn numbers(&self) -> RangeInclusive<u16> {
        self.numbers.clone()
    }

    /// Whether a source could receive a message this reservation covers.
    pub(crate) fn overlaps(&self, source: &Source) -> bool {
        let meets = |reserved: Option<u8>, wanted: Option<u8>| match (reserved, wanted) {
            (Some(reserved), Some(wanted)) => reserved == wanted,
            _ => true,
        };
        matches!(source.control, Control::Nrpn(number) if self.numbers.contains(&number))
            && meets(self.group, source.group)
            && meets(self.channel, source.channel)
    }
}
