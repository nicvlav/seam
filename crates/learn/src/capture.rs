//! The learn flow: arm a target, stage a message on the audio thread, commit
//! it on the UI poll.
//!
//! The UI thread stores the arm: a generation, new at every arm, and what
//! the arm takes. The audio thread reads the arm once per block and stores
//! each legal message it captures with that generation. A message staged
//! for one arm therefore never commits to the next, even when a block read
//! the arm just before a re-arm.
//!
//! A staged capture is in one of three states, its `Resolution`:
//!
//! | State | What it is | What ends it |
//! | --- | --- | --- |
//! | Fine | Final: a note, a MIDI 2.0 message, a MIDI 1.0 controller other than a lone low byte, or an NRPN whose low byte arrived. | The next poll commits it. |
//! | Coarse | Final: an NRPN whose sender sends no low byte. | The next poll commits it, as 7-bit. |
//! | Waiting | Half of a pair: an NRPN high byte, or a MIDI 1.0 controller 32 to 63 with no high byte just before it. | Its other half or a newer capture replaces it. A poll commits it as it stands once an earlier poll found it unchanged and a whole block after the one that staged it has ended. |
//!
//! [`Learn::poll`](crate::Learn::poll) states which half of a pair is learned
//! and when, and the open question for senders that send only low bytes.
//!
//! The audio thread counts its blocks as they begin and end, so a poll can tell
//! whether a whole block ran after the one that staged a capture. Polls alone,
//! with no block between them, never settle a waiting capture: at blocks longer
//! than a UI tick, two polls can run before the block that brings the other
//! half.

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
use std::ops::{BitOr, RangeInclusive};
use std::sync::Arc;

#[cfg(loom)]
use loom::sync::atomic::{AtomicU64, Ordering};
#[cfg(not(loom))]
use std::sync::atomic::{AtomicU64, Ordering};

use crate::field::Field;
use crate::parse::{Message, Value};
use crate::reservation::{CcReservation, NrpnReservation};
use crate::{Control, LearnError, Mapping, Source, TargetId};

/// What an arm takes: controllers (control changes and NRPNs), notes, or
/// both.
///
/// # Examples
///
/// ```
/// use seam_learn::{Takes, TargetId};
#[doc = include_str!("../doc/setup.rs")]
/// # use seam_learn::Config;
/// # use seam_learn::Learn;
/// # let (mut learn, mut audio) = Learn::new(&set, Config::default());
/// // A plain button learns from a note or a controller alike.
/// learn.arm(gain, Takes::CONTROLLER | Takes::NOTE)?;
/// let both = Takes::CONTROLLER | Takes::NOTE;
/// assert_eq!(learn.armed(), Some((TargetId::Param(gain), both)));
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct Takes(u8);

impl Takes {
    /// Control changes and NRPNs.
    pub const CONTROLLER: Takes = Takes(1);
    /// Notes: a note-on's velocity drives the target.
    pub const NOTE: Takes = Takes(2);

    /// Returns whether it accepts `control`'s kind: a control change or an
    /// NRPN is a controller, a note is a note.
    ///
    /// # Examples
    ///
    /// ```
    /// use seam_learn::{Control, Takes};
    ///
    /// assert!(Takes::CONTROLLER.accepts(Control::Nrpn(300)));
    /// assert!(!Takes::CONTROLLER.accepts(Control::Note(60)));
    /// assert!((Takes::CONTROLLER | Takes::NOTE).accepts(Control::Note(60)));
    /// ```
    pub fn accepts(self, control: Control) -> bool {
        let kind = match control {
            Control::Cc(_) | Control::Nrpn(_) => Takes::CONTROLLER,
            Control::Note(_) => Takes::NOTE,
        };
        self.0 & kind.0 != 0
    }
}

/// Names the kinds it takes, as in `Takes(CONTROLLER | NOTE)`, rather than its
/// bits.
impl fmt::Debug for Takes {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Takes(")?;
        let mut none = true;
        for (kind, name) in [(Takes::CONTROLLER, "CONTROLLER"), (Takes::NOTE, "NOTE")] {
            if self.0 & kind.0 != 0 {
                if !none {
                    f.write_str(" | ")?;
                }
                f.write_str(name)?;
                none = false;
            }
        }
        f.write_str(if none { "none)" } else { ")" })
    }
}

impl BitOr for Takes {
    type Output = Takes;

    fn bitor(self, other: Takes) -> Takes {
        Takes(self.0 | other.0)
    }
}

/// The channel mode messages, all sound off to poly on
/// ([`Config::reserve_channel_mode`](crate::Config::reserve_channel_mode)).
const CHANNEL_MODE: RangeInclusive<u8> = 120..=127;

/// What capture refuses, fixed when learn is made, and copied to the audio
/// side so a refused message never reaches the slot.
#[derive(Debug, Clone)]
pub(crate) struct Policy {
    pub(crate) channel_mode: bool,
    pub(crate) ccs: Vec<CcReservation>,
    pub(crate) nrpns: Vec<NrpnReservation>,
}

impl Policy {
    /// Whether an arm taking `takes` may capture `message`.
    fn captures(&self, message: &Message, takes: Takes) -> bool {
        if !takes.accepts(message.control) {
            return false;
        }
        let source = exact(message);
        match message.control {
            Control::Cc(number) => {
                !(self.channel_mode && CHANNEL_MODE.contains(&number))
                    && !self
                        .ccs
                        .iter()
                        .any(|r| r.covers(message.group, message.channel, number))
            }
            Control::Nrpn(_) => !self.nrpns.iter().any(|r| r.overlaps(&source)),
            Control::Note(_) => true,
        }
    }
}

/// A message's source on its own group and channel: what a capture learns.
fn exact(message: &Message) -> Source {
    Source {
        control: message.control,
        group: Some(message.group),
        channel: Some(message.channel),
    }
}

/// The words the arm and the capture share, and the block counts that tell a
/// poll whether a whole block ran after the one that staged a capture.
#[derive(Debug, Default)]
pub(crate) struct Slots {
    /// The arm: see [`ArmWord`].
    arm: AtomicU64,
    /// The newest legal message the audio thread captured: see [`Staged`].
    staged: AtomicU64,
    /// How many blocks have begun, counted before a block stages anything.
    begun: AtomicU64,
    /// How many blocks have ended, counted after a block's last stage.
    ended: AtomicU64,
}

/// How far a capture is settled: the states of the module overview's table.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Resolution {
    Fine,
    Coarse,
    /// Half of a pair. An NRPN's low byte follows its high byte, over DIN
    /// often a block later. A MIDI 1.0 controller 32 to 63 is a low byte
    /// whose high byte the knob's next move sends first.
    Waiting,
}

impl Resolution {
    fn bits(self) -> u64 {
        match self {
            Resolution::Fine => 0,
            Resolution::Coarse => 1,
            Resolution::Waiting => 2,
        }
    }

    fn from_bits(bits: u64) -> Option<Resolution> {
        match bits {
            0 => Some(Resolution::Fine),
            1 => Some(Resolution::Coarse),
            2 => Some(Resolution::Waiting),
            _ => None,
        }
    }
}

/// A captured message as the slot holds it.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Staged {
    generation: u32,
    source: Source,
    resolution: Resolution,
}

// The casts below keep a field's value, which its width bounds.
impl Staged {
    // The word, low bits first. Generation 0, which no arm has, is no
    // capture.
    const NUMBER: Field = Field::lowest(14);
    const KIND: Field = Field::above(Self::NUMBER, 2);
    const GROUP: Field = Field::above(Self::KIND, 4);
    const CHANNEL: Field = Field::above(Self::GROUP, 4);
    const RESOLUTION: Field = Field::above(Self::CHANNEL, 2);
    const GENERATION: Field = Field::above(Self::RESOLUTION, 32);

    fn pack(self) -> u64 {
        let Source {
            control,
            group,
            channel,
        } = self.source;
        Self::NUMBER.put(u64::from(control.number()))
            | Self::KIND.put(control.kind())
            | Self::GROUP.put(u64::from(group.unwrap_or(0)))
            | Self::CHANNEL.put(u64::from(channel.unwrap_or(0)))
            | Self::RESOLUTION.put(self.resolution.bits())
            | Self::GENERATION.put(u64::from(self.generation))
    }

    fn unpack(word: u64) -> Option<Staged> {
        let generation = Self::GENERATION.get(word) as u32;
        if generation == 0 {
            return None;
        }
        let number = Self::NUMBER.get(word) as u16;
        Some(Staged {
            generation,
            source: Source {
                control: Control::from_kind(Self::KIND.get(word), number)?,
                group: Some(Self::GROUP.get(word) as u8),
                channel: Some(Self::CHANNEL.get(word) as u8),
            },
            resolution: Resolution::from_bits(Self::RESOLUTION.get(word))?,
        })
    }
}

/// The arm as its word holds it: a generation, new at every arm, and what
/// the arm takes. Generation 0 is nothing armed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ArmWord {
    generation: u32,
    takes: Takes,
}

// The casts below keep a field's value, which its width bounds.
impl ArmWord {
    const TAKES: Field = Field::lowest(8);
    const GENERATION: Field = Field::above(Self::TAKES, 32);

    const DISARMED: ArmWord = ArmWord {
        generation: 0,
        takes: Takes(0),
    };

    fn pack(self) -> u64 {
        Self::TAKES.put(u64::from(self.takes.0)) | Self::GENERATION.put(u64::from(self.generation))
    }

    fn unpack(word: u64) -> ArmWord {
        ArmWord {
            generation: Self::GENERATION.get(word) as u32,
            takes: Takes(Self::TAKES.get(word) as u8),
        }
    }
}

/// One block's offers, which only [`Stager::block`] hands out.
#[derive(Debug)]
pub(crate) struct Offers<'a>(&'a mut Stager);

impl Offers<'_> {
    /// Stages `message` if the arm takes it and nothing blocks it, so a
    /// refused message never hides an earlier legal one.
    pub(crate) fn offer(&mut self, message: &Message) {
        self.0.offer(message);
    }
}

/// The audio thread's side of capture. Allocation-free.
#[derive(Debug)]
pub(crate) struct Stager {
    slots: Arc<Slots>,
    policy: Policy,
    /// The arm read at this block's start.
    generation: u32,
    takes: Takes,
    /// An NRPN high byte waiting for a low byte.
    waiting: Option<Source>,
    /// The NRPN this arm has already settled, [`Resolution::Fine`] or
    /// [`Resolution::Coarse`], so its later high bytes stay settled.
    settled_nrpn: Option<(Source, Resolution)>,
    /// The source this arm staged last, so a controller 32 to 63 that comes
    /// right after its high byte learns the high byte.
    newest: Option<Source>,
}

impl Stager {
    pub(crate) fn new(slots: Arc<Slots>, policy: Policy) -> Stager {
        Stager {
            slots,
            policy,
            generation: 0,
            takes: Takes(0),
            waiting: None,
            settled_nrpn: None,
            newest: None,
        }
    }

    /// Runs one block's offers between its two counts: counts the block as
    /// begun and reads the arm, runs `offers`, then counts the block as
    /// ended.
    ///
    /// Only [`Offers`] can offer, and only this hands one out, so no offer
    /// falls outside the counts. A block that panics is never counted as
    /// ended, so a waiting half waits for the next whole block.
    pub(crate) fn block(&mut self, offers: impl FnOnce(&mut Offers<'_>)) {
        self.begin_block();
        offers(&mut Offers(self));
        self.end_block();
    }

    /// Counts the block and reads the arm, before any offer.
    fn begin_block(&mut self) {
        // Relaxed is enough. A poll reads this count only after a staged
        // word, and a word this block stages is released after the count, so
        // the word's acquire carries the count with it.
        self.slots.begun.fetch_add(1, Ordering::Relaxed);
        let ArmWord { generation, takes } = ArmWord::unpack(self.slots.arm.load(Ordering::Acquire));
        if generation != self.generation {
            self.waiting = None;
            self.settled_nrpn = None;
            self.newest = None;
        }
        self.generation = generation;
        self.takes = takes;
    }

    /// Counts the block as ended, after its last offer.
    fn end_block(&self) {
        // Release: a poll that reads this count before the staged word then
        // reads every word this block staged, or a newer one.
        self.slots.ended.fetch_add(1, Ordering::Release);
    }

    /// What [`Offers::offer`] does.
    fn offer(&mut self, message: &Message) {
        if self.generation == 0 || !self.policy.captures(message, self.takes) {
            return;
        }
        let source = self.pair_low_byte(message);
        match message.value {
            Value::Nrpn { low: None, .. } => {
                if let Some((known, resolution)) = self.settled_nrpn
                    && known == source
                {
                    // Newer than any other waiting high byte, which must not
                    // settle over it.
                    self.waiting = None;
                    self.stage(source, resolution);
                } else if self.waiting == Some(source) {
                    // A second high byte with no low byte between: the
                    // sender sends none.
                    self.settle(source, Resolution::Coarse);
                } else {
                    // Its low byte may still come. Until the poll settles
                    // it, it hides the older capture it replaced.
                    self.waiting = Some(source);
                    self.stage(source, Resolution::Waiting);
                }
            }
            Value::Nrpn { low: Some(_), .. } => self.settle(source, Resolution::Fine),
            // A low byte whose high byte did not come just before it. The
            // arm may have landed between the halves, so the poll waits for
            // the knob's next move, which sends the high byte first.
            Value::Seven(_) if is_low_byte(source.control) => {
                self.waiting = None;
                self.stage(source, Resolution::Waiting);
            }
            _ => {
                self.waiting = None;
                self.stage(source, Resolution::Fine);
            }
        }
    }

    /// The source `message` learns.
    ///
    /// A MIDI 1.0 controller 32 to 63 is the low byte of a 14-bit pair whose
    /// high byte is 32 below it. When that high byte is the capture just before
    /// it, the pair learns the high byte, because the low byte alone would bind
    /// half a value. Only the capture just before counts, so a knob 32 above
    /// moved after another capture is learned as itself.
    fn pair_low_byte(&self, message: &Message) -> Source {
        let source = exact(message);
        if let (Control::Cc(number @ 32..=63), Value::Seven(_)) = (message.control, message.value) {
            #[expect(
                clippy::arithmetic_side_effects,
                reason = "the pattern bounds the number to 32 to 63"
            )]
            let high = Source {
                control: Control::Cc(number - 32),
                ..source
            };
            if self.newest == Some(high) {
                return high;
            }
        }
        source
    }

    fn settle(&mut self, source: Source, resolution: Resolution) {
        self.waiting = None;
        self.settled_nrpn = Some((source, resolution));
        self.stage(source, resolution);
    }

    fn stage(&mut self, source: Source, resolution: Resolution) {
        self.newest = Some(source);
        let staged = Staged {
            generation: self.generation,
            source,
            resolution,
        };
        self.slots.staged.store(staged.pack(), Ordering::Release);
    }
}

/// Whether `control` is a controller 32 to 63, the low byte of a 14-bit
/// pair in MIDI 1.0.
fn is_low_byte(control: Control) -> bool {
    matches!(control, Control::Cc(32..=63))
}

/// The UI thread's side of capture.
#[derive(Debug)]
pub(crate) struct Arm {
    slots: Arc<Slots>,
    generation: u32,
    armed: Option<(TargetId, Takes)>,
    /// The waiting capture an earlier poll found, so a later poll can tell a
    /// lone NRPN high byte from one whose low byte is still on its way.
    unsettled: Option<Unsettled>,
}

/// A waiting capture as the poll that first found it saw it.
#[derive(Debug, Clone, Copy)]
struct Unsettled {
    /// The staged word.
    word: u64,
    /// How many blocks had begun, read after the word, so the count includes
    /// the block that staged it.
    begun: u64,
}

impl Arm {
    pub(crate) fn new(slots: Arc<Slots>) -> Arm {
        Arm {
            slots,
            generation: 0,
            armed: None,
            unsettled: None,
        }
    }

    pub(crate) fn armed(&self) -> Option<(TargetId, Takes)> {
        self.armed
    }

    /// The words the audio side shares.
    pub(crate) fn slots(&self) -> &Arc<Slots> {
        &self.slots
    }

    /// Arms `target` with a new generation, so nothing staged before counts.
    pub(crate) fn arm(&mut self, target: TargetId, takes: Takes) {
        self.generation = self.generation.wrapping_add(1).max(1);
        self.armed = Some((target, takes));
        self.unsettled = None;
        let word = ArmWord {
            generation: self.generation,
            takes,
        };
        self.slots.arm.store(word.pack(), Ordering::Release);
    }

    pub(crate) fn disarm(&mut self) -> bool {
        self.unsettled = None;
        self.slots
            .arm
            .store(ArmWord::DISARMED.pack(), Ordering::Release);
        self.armed.take().is_some()
    }

    /// The settled capture for the current arm, if one is staged.
    ///
    /// A waiting capture settles once [`has_waited`](Self::has_waited) says
    /// so: an NRPN high byte as coarse, a controller 32 to 63 as itself.
    pub(crate) fn capture(&mut self) -> Option<(TargetId, Mapping)> {
        let (target, _) = self.armed?;
        // Acquire, and before the word: every block this count shows ended
        // staged before it counted, so the word read next is the newest of
        // those blocks' stages or a later one.
        let ended = self.slots.ended.load(Ordering::Acquire);
        let word = self.slots.staged.load(Ordering::Acquire);
        let staged = Staged::unpack(word)?;
        if staged.generation != self.generation {
            return None;
        }
        if staged.resolution == Resolution::Waiting && !self.has_waited(word, ended) {
            return None;
        }
        let mut mapping = Mapping::new(staged.source);
        mapping.coarse = match staged.resolution {
            Resolution::Fine => false,
            Resolution::Coarse => true,
            Resolution::Waiting => matches!(staged.source.control, Control::Nrpn(_)),
        };
        Some((target, mapping))
    }

    /// Whether the waiting capture `word` has waited out a whole block: an
    /// earlier poll found it, and a block after the one that staged it had
    /// `ended` before this poll read the word. A first sighting is noted.
    fn has_waited(&mut self, word: u64, ended: u64) -> bool {
        if let Some(seen) = self.unsettled
            && seen.word == word
        {
            return ended > seen.begun;
        }
        // Read after the word. The block that staged it counted itself
        // before staging, and the word's acquire carries that count here.
        let begun = self.slots.begun.load(Ordering::Relaxed);
        self.unsettled = Some(Unsettled { word, begun });
        false
    }
}

/// What a poll did with the arm.
///
/// # Examples
///
/// ```
/// use seam_learn::{Config, Learn, Learned, LearnError, Takes};
#[doc = include_str!("../doc/setup.rs")]
/// // A table that holds no mappings refuses the one the arm learned.
/// let mut config = Config::default();
/// config.max_mappings = 0;
/// let (mut learn, mut audio) = Learn::new(&set, config);
///
/// learn.arm(gain, Takes::CONTROLLER)?;
/// audio.process(&block(&[(7, 127)])); // controller 7 moves
/// match learn.poll() {
///     Some(Learned::Refused { error, .. }) => assert_eq!(error, LearnError::Full),
///     other => panic!("expected a refusal, got {other:?}"),
/// }
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
#[derive(Debug, Clone, Copy, PartialEq)]
#[non_exhaustive]
pub enum Learned {
    /// The capture became the target's mapping, through the identity curve,
    /// on the group and channel it arrived on. The arm is over.
    Mapped {
        /// The armed target.
        target: TargetId,
        /// Its new mapping.
        mapping: Mapping,
    },
    /// The mapping was refused, the table full say, so nothing changed; the
    /// arm is over.
    Refused {
        /// The armed target.
        target: TargetId,
        /// Why.
        error: LearnError,
    },
}

#[cfg(test)]
mod tests;
