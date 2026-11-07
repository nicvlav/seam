//! The one authoritative mapping table, on the UI thread.

mod session;

use std::collections::BTreeMap;
use std::fmt;
use std::sync::Arc;

use seam_params::ParamSet;
use seam_session::{Registration, Section};

use crate::capture::{Arm, Learned, Policy, Slots, Stager, Takes};
use crate::saved::{decode, encode};
use crate::snapshot::{Publisher, snapshot};
use crate::table::{Route, Table};
use crate::target::{BURST_IDLE_TICKS, Staging};
use crate::{
    CcReservation, Curve, CustomId, LearnAudio, NrpnReservation, Source, Target, TargetId,
};

/// The mapping ceiling unless a plugin sets its own.
pub const DEFAULT_MAX_MAPPINGS: usize = 4096;

/// What drives one target: a source, and the curve its value passes.
///
/// More fields may come, so outside this crate [`new`](Self::new) and the
/// setters make one, and its fields stay readable.
///
/// # Examples
///
/// ```
/// use seam_learn::{Control, Curve, Mapping, Source};
///
/// // Controller 7 drives the target backwards, over the upper half.
/// let source = Source::any(Control::Cc(7));
/// let mapping = Mapping::new(source).with_curve(Curve::new(1.0, 0.5, 1.0)?.inverted());
/// assert_eq!(mapping.source, source);
/// assert!(!mapping.coarse);
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
#[derive(Debug, Clone, Copy, PartialEq)]
#[non_exhaustive]
pub struct Mapping {
    /// Where it listens.
    pub source: Source,
    /// What it does to the value.
    pub curve: Curve,
    /// For a MIDI 1.0 NRPN: whether the capture saw no low byte (controller
    /// 38).
    ///
    /// A coarse binding reads the high byte as 7-bit and ignores low bytes,
    /// so a sender that never sends the low byte reaches 1.0. Otherwise the
    /// value is 14-bit, and a high byte alone reads as `high << 7`, as MIDI
    /// 1.0 says. Ignored by every other source, MIDI 2.0's assignable
    /// controllers included.
    pub coarse: bool,
}

impl Mapping {
    /// Creates a mapping of `source` through the identity curve, 14-bit for an
    /// NRPN, as the [`Mapping`] example shows.
    pub const fn new(source: Source) -> Self {
        Mapping {
            source,
            curve: Curve::IDENTITY,
            coarse: false,
        }
    }

    /// Returns the mapping with its value passed through `curve`.
    #[must_use]
    pub const fn with_curve(mut self, curve: Curve) -> Mapping {
        self.curve = curve;
        self
    }

    /// Returns the mapping reading an NRPN as [`coarse`](Self::coarse).
    ///
    /// # Examples
    ///
    /// ```
    /// use seam_learn::{Control, Mapping, Source};
    ///
    /// // A sender that never sends the low byte still reaches the top.
    /// let mapping = Mapping::new(Source::any(Control::Nrpn(300)));
    /// assert!(!mapping.coarse);
    /// assert!(mapping.as_coarse().coarse);
    /// ```
    #[must_use]
    pub const fn as_coarse(mut self) -> Mapping {
        self.coarse = true;
        self
    }
}

/// One plugin instance's learn policy, fixed when learn is made. Two
/// instances in one process share nothing.
///
/// More fields may come, so outside this crate a config starts from
/// [`Config::default`] and sets the fields it changes.
///
/// # Examples
///
/// ```
/// use seam_learn::{Config, Learn};
#[doc = include_str!("../doc/setup.rs")]
/// // A small instrument: at most 64 mappings, and the panic button stays
/// // learnable.
/// let mut config = Config::default();
/// config.max_mappings = 64;
/// config.reserve_channel_mode = false;
/// let (learn, _audio) = Learn::new(&set, config.clone());
/// assert_eq!(learn.config(), &config);
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Config {
    /// The most mappings at once. A new mapping past it is refused;
    /// re-pointing a mapped target is always allowed.
    pub max_mappings: usize,
    /// NRPN ranges the plugin's own protocol owns ([`NrpnReservation`]).
    pub reserved_nrpns: Vec<NrpnReservation>,
    /// Controller ranges learn never captures ([`CcReservation`]).
    pub reserved_ccs: Vec<CcReservation>,
    /// Whether learn refuses to capture the channel mode messages, 120 to
    /// 127, so a panic button pressed while armed is not learned. On by
    /// default.
    pub reserve_channel_mode: bool,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            max_mappings: DEFAULT_MAX_MAPPINGS,
            reserved_nrpns: Vec::new(),
            reserved_ccs: Vec::new(),
            reserve_channel_mode: true,
        }
    }
}

/// Why a mapping or a registration was refused. A refused call changes
/// nothing.
///
/// # Examples
///
/// ```
/// use seam_learn::{Config, Control, Learn, LearnError, Mapping, Source};
#[doc = include_str!("../doc/setup.rs")]
/// let (mut learn, _audio) = Learn::new(&set, Config::default());
///
/// // Controller 6 belongs to the NRPN parser; 200 is no controller at all.
/// let parser_owned = Mapping::new(Source::any(Control::Cc(6)));
/// let blocked = learn.map(gain, parser_owned);
/// assert!(matches!(blocked, Err(LearnError::Blocked(_))));
/// let out_of_range = Mapping::new(Source::any(Control::Cc(200)));
/// let bad = learn.map(gain, out_of_range);
/// assert!(matches!(bad, Err(LearnError::BadSource(_))));
/// let taken = learn.register("gain", Box::new(Fader));
/// assert_eq!(taken, Err(LearnError::KeyTaken));
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
#[derive(Debug, Clone, Copy, PartialEq)]
#[non_exhaustive]
pub enum LearnError {
    /// No such target: a parameter index past this set's, or a custom
    /// target not registered here.
    ///
    /// Only a parameter's index is checked: a
    /// [`ParamId`](seam_params::ParamId) carries no set identity, so one from
    /// another set with an index in range names this set's parameter there.
    UnknownTarget(TargetId),
    /// A number in the source is out of range for its kind.
    BadSource(Source),
    /// The source can never be bound: a controller the parser owns, as
    /// [`Control::Cc`](crate::Control::Cc) lists them, or an NRPN the plugin
    /// reserved.
    Blocked(Source),
    /// The table holds [`Config::max_mappings`] mappings and the target has
    /// none to re-point.
    Full,
    /// A custom target's key is empty.
    EmptyKey,
    /// A parameter or another custom target has this key.
    KeyTaken,
}

impl fmt::Display for LearnError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LearnError::UnknownTarget(target) => write!(f, "{target:?} is not a target here"),
            LearnError::BadSource(source) => write!(f, "{source:?} is out of range"),
            LearnError::Blocked(source) => write!(f, "{source:?} can never be bound"),
            LearnError::Full => write!(f, "the mapping table is full"),
            LearnError::EmptyKey => write!(f, "a target key must not be empty"),
            LearnError::KeyTaken => write!(f, "another target has this key"),
        }
    }
}

impl std::error::Error for LearnError {}

/// A target learn owns and writes on the UI thread.
struct Custom {
    key: String,
    target: Box<dyn Target>,
    /// What the table hands the audio thread while the target is mapped,
    /// and `None` while it is not: a fresh one at each mapping, a re-point
    /// included, so no value an old source staged lands.
    staging: Option<Arc<Staging>>,
    /// Poll ticks since the open burst's last value; `None` with no burst
    /// open.
    burst: Option<u32>,
}

impl fmt::Debug for Custom {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Custom")
            .field("key", &self.key)
            .field("mapped", &self.staging.is_some())
            .field("burst", &self.burst)
            .finish_non_exhaustive()
    }
}

impl Custom {
    /// One poll tick of the target's burst, as [`Target`] describes it. An
    /// unmapped target's burst closes here if a panicking `end` left it
    /// open.
    fn tick(&mut self) {
        let Some(staging) = &self.staging else {
            self.close();
            return;
        };
        if self.target.held() {
            // The user owns it: the value waits for them to let go.
            self.close();
            return;
        }
        match staging.take() {
            Some(value) => {
                if self.burst.is_none() {
                    self.target.begin();
                }
                self.burst = Some(0);
                self.target.apply(value);
            }
            None => {
                if let Some(idle) = &mut self.burst {
                    *idle += 1;
                    if *idle >= BURST_IDLE_TICKS {
                        self.close();
                    }
                }
            }
        }
    }

    /// Closes the open burst, once, whether or not the target is mapped.
    fn close(&mut self) {
        if self.burst.take().is_some() {
            self.target.end();
        }
    }
}

/// The UI thread's side of learn: the one authoritative mapping table, the
/// learn flow, and the targets it writes on the UI thread.
///
/// Call [`poll`](Self::poll) once per UI tick, and [`LearnAudio::process`]
/// once per audio block on the audio side [`Learn::new`] returned.
///
/// Every mapping change publishes a new table, which the audio thread takes
/// at its next block. A target has at most one source, and one source may
/// drive any number of targets. Only mapped targets take a slot, so the
/// audio thread's work follows the mappings, not the targets.
///
/// # Examples
///
/// Arm a parameter, play a controller, and let the next UI tick commit it:
///
/// ```
/// use seam_learn::{Config, Learn, Learned, Takes};
/// use seam_params::{Kind, ParamDecl, ParamSetBuilder, Range};
/// use seam_ump::{ChannelVoice, EventList, Voice};
///
/// let mut builder = ParamSetBuilder::new();
/// builder.add(ParamDecl::new("gain", "Gain", Kind::continuous(0.0, 1.0, 0.0, None), 0.0))?;
/// let set = builder.build();
/// let gain = set.ids().next().expect("one parameter");
/// let (mut learn, mut audio) = Learn::new(&set, Config::default());
///
/// // One block of MIDI 1.0 control changes, as (controller, value) pairs.
/// let block = |changes: &[(u8, u8)]| {
///     let mut events = EventList::with_capacity(changes.len());
///     for &(controller, value) in changes {
///         let voice = Voice::Midi1ControlChange { controller, value };
///         events.push(ChannelVoice { group: 0, channel: 0, voice }.event(0));
///     }
///     events
/// };
///
/// learn.arm(gain, Takes::CONTROLLER)?;
/// audio.process(&block(&[(7, 127)])); // Audio thread, once per block.
/// let learned = learn.poll(); // UI thread, once per tick.
/// assert!(matches!(learned, Some(Learned::Mapped { .. })));
///
/// // From now on controller 7 writes the parameter in its own block.
/// audio.process(&block(&[(7, 127)]));
/// assert_eq!(set.value(gain), 1.0);
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
#[derive(Debug)]
pub struct Learn {
    set: Arc<ParamSet>,
    config: Config,
    mappings: BTreeMap<TargetId, Mapping>,
    customs: BTreeMap<CustomId, Custom>,
    next_custom: u64,
    /// Entries for targets not registered yet, by key, which take no slot
    /// under the ceiling.
    ///
    /// They are restored ones kept as they were saved, so a save writes them
    /// back unchanged, and the mappings of detached targets.
    held: BTreeMap<String, Section>,
    /// The session registration that saves the section, once attached.
    registration: Option<Registration>,
    publisher: Publisher,
    arm: Arm,
}

impl Learn {
    /// Creates learn for the parameters of `set`, and the audio side it
    /// publishes to. The [`Learn`] example makes both.
    ///
    /// Thread: any but audio.
    pub fn new(set: &Arc<ParamSet>, config: Config) -> (Learn, LearnAudio) {
        let (publisher, reader) = snapshot();
        let slots = Arc::new(Slots::default());
        let audio = LearnAudio::new(
            set.audio(),
            reader,
            Stager::new(Arc::clone(&slots), policy(&config)),
        );
        let learn = Learn {
            set: Arc::clone(set),
            config,
            mappings: BTreeMap::new(),
            customs: BTreeMap::new(),
            next_custom: 0,
            held: BTreeMap::new(),
            registration: None,
            publisher,
            arm: Arm::new(slots),
        };
        (learn, audio)
    }

    /// Returns the policy it was made with.
    pub fn config(&self) -> &Config {
        &self.config
    }

    /// Replaces the audio side with a fresh one, for a new activation.
    ///
    /// A VST3 host reactivates at every sample-rate or block-size change, and
    /// each activation's processor needs its own [`LearnAudio`]. The new one
    /// reads the current mappings from its first block. The previous one
    /// keeps the table it last read, sees no later change, and is dropped off
    /// the audio thread like any other.
    ///
    /// Thread: any but audio.
    ///
    /// # Examples
    ///
    /// ```
    /// use seam_learn::{Control, Mapping, Source};
    #[doc = include_str!("../doc/setup.rs")]
    /// # use seam_learn::Config;
    /// # use seam_learn::Learn;
    /// # let (mut learn, mut audio) = Learn::new(&set, Config::default());
    /// learn.map(gain, Mapping::new(Source::any(Control::Cc(7))))?;
    ///
    /// // The host reactivates the plugin: the new processor gets its own audio
    /// // side, which starts from the mappings made so far.
    /// let mut reactivated = learn.replace_audio();
    /// reactivated.process(&block(&[(7, 127)])); // controller 7 moves
    /// assert_eq!(set.value(gain), 1.0);
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    pub fn replace_audio(&mut self) -> LearnAudio {
        let (publisher, reader) = snapshot();
        self.publisher = publisher;
        self.publish();
        let stager = Stager::new(Arc::clone(self.arm.slots()), policy(&self.config));
        LearnAudio::new(self.set.audio(), reader, stager)
    }

    /// Registers a target learn writes on the UI thread, under `key`, its
    /// identity in the saved mappings. Learn owns it from now on.
    ///
    /// The target starts unmapped, unless a restore or a detach held an entry
    /// for `key`. That entry lands now under the checks a restore makes, or
    /// drops if they refuse it. [`Target`] shows a target and how learn
    /// writes it.
    ///
    /// # Errors
    ///
    /// Returns [`LearnError::EmptyKey`] for an empty key, and
    /// [`LearnError::KeyTaken`] for one a parameter or another target has.
    pub fn register(&mut self, key: &str, target: Box<dyn Target>) -> Result<CustomId, LearnError> {
        if key.is_empty() {
            return Err(LearnError::EmptyKey);
        }
        if self.target_for(key).is_some() {
            return Err(LearnError::KeyTaken);
        }
        let id = CustomId(self.next_custom);
        self.next_custom += 1;
        self.customs.insert(
            id,
            Custom {
                key: key.to_owned(),
                target,
                staging: None,
                burst: None,
            },
        );
        if let Some(entry) = self.held.remove(key) {
            let target = TargetId::Custom(id);
            if let Some((_, mapping)) = decode(&entry)
                && self.check(target, &mapping).is_ok()
            {
                self.insert(target, mapping);
                self.publish();
            }
            self.saved_changed();
        }
        Ok(id)
    }

    /// Detaches a registered target and returns whether it was registered.
    /// Detaching ends an arm on the target, closes its open burst once, and
    /// drops it.
    ///
    /// Its mapping, if it has one, is held under its key, as a restore holds
    /// an entry for a target not registered yet, and lands when the key
    /// [registers](Self::register) again. To drop a target and its mapping
    /// for good, [`unmap`](Self::unmap) it first. A block already in flight
    /// can still stage a value for it, which nothing reads.
    ///
    /// # Examples
    ///
    /// ```
    /// use seam_learn::{Config, Control, Learn, Mapping, Source};
    #[doc = include_str!("../doc/setup.rs")]
    /// let (mut learn, _audio) = Learn::new(&set, Config::default());
    /// let fader = learn.register("fader", Box::new(Fader))?;
    /// learn.map(fader, Mapping::new(Source::any(Control::Cc(7))))?;
    ///
    /// // The mapping is held under the key, ready for the next fader.
    /// assert!(learn.detach(fader));
    /// assert!(learn.mapping(fader).is_none());
    /// assert!(!learn.detach(fader), "already gone");
    /// let again = learn.register("fader", Box::new(Fader))?;
    /// assert!(learn.mapping(again).is_some());
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    pub fn detach(&mut self, id: CustomId) -> bool {
        let Some(mut custom) = self.customs.remove(&id) else {
            return false;
        };
        let target = TargetId::Custom(id);
        // Mapped or not, an arm on it ends.
        self.edited(target);
        // The table is settled before the target's own code runs, so a
        // panicking `end` leaves no mapping whose target is gone. What a
        // save writes is unchanged, since the held entry is the same
        // encoding, so the session needs no new capture.
        if let Some(mapping) = self.mappings.remove(&target) {
            self.held
                .insert(custom.key.clone(), encode(&custom.key, &mapping));
            self.publish();
        }
        custom.close();
        true
    }

    /// Arms `target`: the next legal message the audio thread sees becomes its
    /// mapping at a later [`poll`](Self::poll).
    ///
    /// The last arm wins, and nothing staged before it counts. Capture
    /// refuses a kind the arm does not take, a reserved controller or NRPN,
    /// and a channel mode message unless the [`Config`] allows it. A refused
    /// message leaves the target armed and never hides an earlier legal one.
    ///
    /// # Errors
    ///
    /// Returns [`LearnError::UnknownTarget`] when `target` is not a target
    /// here.
    ///
    /// # Examples
    ///
    /// ```
    /// use seam_learn::{Config, Learn, Takes};
    #[doc = include_str!("../doc/setup.rs")]
    /// let (mut learn, _audio) = Learn::new(&set, Config::default());
    /// let fader = learn.register("fader", Box::new(Fader))?;
    ///
    /// learn.arm(gain, Takes::CONTROLLER)?;
    /// assert_eq!(learn.armed(), Some((gain.into(), Takes::CONTROLLER)));
    ///
    /// // The last arm wins.
    /// learn.arm(fader, Takes::NOTE)?;
    /// assert_eq!(learn.armed(), Some((fader.into(), Takes::NOTE)));
    ///
    /// // Cancelling leaves the mappings alone and says whether one was armed.
    /// assert!(learn.cancel());
    /// assert!(!learn.cancel());
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    pub fn arm(&mut self, target: impl Into<TargetId>, takes: Takes) -> Result<(), LearnError> {
        let target = target.into();
        if !self.known(target) {
            return Err(LearnError::UnknownTarget(target));
        }
        self.arm.arm(target, takes);
        Ok(())
    }

    /// Cancels the arm, leaving the mappings alone. Returns whether one was
    /// armed, as the [`arm`](Self::arm) example shows.
    pub fn cancel(&mut self) -> bool {
        self.arm.disarm()
    }

    /// Returns the armed target and what it takes, or `None` with no arm.
    pub fn armed(&self) -> Option<(TargetId, Takes)> {
        self.arm.armed()
    }

    /// Runs one UI tick of learn: call it once per tick.
    ///
    /// First it commits the capture the audio thread staged for the current
    /// arm, once it has settled, and the arm is over; the [`Learned`] it
    /// returns says how that went.
    ///
    /// A capture settles at once, except half of a 14-bit pair, which waits for
    /// the other half. The wait ends at a poll that finds the half unchanged
    /// since an earlier poll, once a whole audio block after the one that
    /// brought the half has ended, so it lasts at least a UI tick and a block.
    /// A half waits while the host runs no blocks, since its other half arrives
    /// in one.
    ///
    /// - An NRPN high byte waits for its low byte, which upgrades it to 14-bit.
    ///   The high byte is learned as coarse when the wait ends, or at once
    ///   when a second high byte arrives without a low one.
    /// - A MIDI 1.0 controller 32 to 63 is the low byte of a 14-bit pair.
    ///   Right after its high byte, with no other capture between, it learns
    ///   the high byte. Otherwise it waits in the same way, and its high byte
    ///   wins if that comes before the wait ends. If not, the low byte is
    ///   learned as itself.
    ///
    /// Open: MIDI 1.0 lets a 14-bit sender send only the low byte for a fine
    /// move, and a slow turn of such a knob is learned as its low byte.
    /// Controllers 32 to 63 also serve as plain 7-bit controllers, so capture
    /// cannot tell which it sees and does not guess.
    ///
    /// Then it writes each mapped [`Target`] the newest value the audio
    /// thread staged for it, one gesture per burst.
    ///
    /// Thread: UI.
    ///
    /// # Examples
    ///
    /// The [`Learn`] example shows an arm committed at the next poll, and the
    /// [`Target`] example a registered target written one burst at a time.
    /// Half of a pair waits:
    ///
    /// ```
    /// use seam_learn::{Config, Control, Learn, Learned, Takes};
    #[doc = include_str!("../doc/setup.rs")]
    /// let (mut learn, mut audio) = Learn::new(&set, Config::default());
    /// learn.arm(gain, Takes::CONTROLLER)?;
    ///
    /// // NRPN 300's address (controllers 99 and 98), then its high byte (6).
    /// audio.process(&block(&[(99, 2), (98, 44), (6, 64)]));
    /// assert!(learn.poll().is_none(), "the high byte waits for its low byte");
    /// assert!(learn.poll().is_none(), "no block ran since the last poll");
    ///
    /// // The low byte (38) comes in the next block: a 14-bit mapping.
    /// audio.process(&block(&[(38, 5)]));
    /// match learn.poll() {
    ///     Some(Learned::Mapped { mapping, .. }) => {
    ///         assert_eq!(mapping.source.control, Control::Nrpn(300));
    ///         assert!(!mapping.coarse);
    ///     }
    ///     other => panic!("expected a mapping, got {other:?}"),
    /// }
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    pub fn poll(&mut self) -> Option<Learned> {
        let learned = self.commit();
        self.customs.values_mut().for_each(Custom::tick);
        learned
    }

    fn commit(&mut self) -> Option<Learned> {
        let (target, mapping) = self.arm.capture()?;
        self.arm.disarm();
        Some(match self.map(target, mapping) {
            Ok(_) => Learned::Mapped { target, mapping },
            Err(error) => Learned::Refused { target, error },
        })
    }

    /// Maps `target`, replacing its mapping if it has one, and publishes.
    ///
    /// Returns the mapping it replaced. A source that capture refuses, such
    /// as a reserved controller, is accepted here: naming it is intent.
    /// Mapping the armed target cancels the arm, so a later message cannot
    /// overwrite the change. A UI-tier target newly mapped starts with
    /// nothing staged.
    ///
    /// # Errors
    ///
    /// Returns a [`LearnError`] when the target is unknown, the source is out
    /// of range or can never be bound, or the table is full; nothing changes.
    ///
    /// # Examples
    ///
    /// ```
    /// use seam_learn::{Control, Mapping, Source};
    #[doc = include_str!("../doc/setup.rs")]
    /// # use seam_learn::Config;
    /// # use seam_learn::Learn;
    /// # let (mut learn, mut audio) = Learn::new(&set, Config::default());
    /// let source = Source::any(Control::Cc(7));
    /// assert_eq!(learn.map(gain, Mapping::new(source))?, None);
    ///
    /// // Mapping the same target again replaces the first and hands it back.
    /// let other = Mapping::new(Source::any(Control::Cc(11)));
    /// let replaced = learn.map(gain, other)?;
    /// assert_eq!(replaced, Some(Mapping::new(source)));
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    pub fn map(
        &mut self,
        target: impl Into<TargetId>,
        mapping: Mapping,
    ) -> Result<Option<Mapping>, LearnError> {
        let target = target.into();
        self.check(target, &mapping)?;
        self.edited(target);
        let replaced = self.insert(target, mapping);
        self.publish();
        self.saved_changed();
        Ok(replaced)
    }

    /// Puts `mapping` in the table, unchecked and unpublished.
    fn insert(&mut self, target: TargetId, mapping: Mapping) -> Option<Mapping> {
        if let TargetId::Custom(id) = target
            && let Some(custom) = self.customs.get_mut(&id)
        {
            custom.staging = Some(Arc::new(Staging::default()));
        }
        self.mappings.insert(target, mapping)
    }

    /// Removes `target`'s mapping and publishes. Returns it, if there was one;
    /// with none, nothing is published.
    ///
    /// Unmapping the armed target cancels the arm. A UI-tier target's open
    /// burst closes, once, and the poll never writes it again.
    ///
    /// # Examples
    ///
    /// ```
    /// use seam_learn::{Config, Control, Learn, Mapping, Source};
    #[doc = include_str!("../doc/setup.rs")]
    /// let (mut learn, _audio) = Learn::new(&set, Config::default());
    /// let mapping = Mapping::new(Source::any(Control::Cc(7)));
    /// learn.map(gain, mapping)?;
    ///
    /// assert_eq!(learn.unmap(gain), Some(mapping));
    /// assert_eq!(learn.unmap(gain), None); // nothing left to remove
    /// assert!(learn.is_empty());
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    pub fn unmap(&mut self, target: impl Into<TargetId>) -> Option<Mapping> {
        let target = target.into();
        self.edited(target);
        let removed = self.mappings.remove(&target)?;
        // The poll never writes the target again.
        if let Some(custom) = self.custom(target) {
            custom.staging = None;
        }
        self.publish();
        self.saved_changed();
        // Last, so a panicking `end` finds the table published.
        if let Some(custom) = self.custom(target) {
            custom.close();
        }
        Some(removed)
    }

    /// The registered custom target `target` names, if it names one.
    fn custom(&mut self, target: TargetId) -> Option<&mut Custom> {
        match target {
            TargetId::Custom(id) => self.customs.get_mut(&id),
            TargetId::Param(_) => None,
        }
    }

    /// Returns the key `target` saves under: a parameter's key, or the key a
    /// custom target registered with. `None` for a target not known here.
    pub fn key(&self, target: impl Into<TargetId>) -> Option<&str> {
        match target.into() {
            TargetId::Param(id) => (id.index() < self.set.len()).then(|| self.set.decl(id).key),
            TargetId::Custom(id) => self.customs.get(&id).map(|custom| custom.key.as_str()),
        }
    }

    /// Cancels the arm when `target` is armed.
    fn edited(&mut self, target: TargetId) {
        if self.arm.armed().is_some_and(|(armed, _)| armed == target) {
            self.arm.disarm();
        }
    }

    /// Returns `target`'s mapping, or `None` while it is unmapped.
    pub fn mapping(&self, target: impl Into<TargetId>) -> Option<&Mapping> {
        self.mappings.get(&target.into())
    }

    /// Returns every mapping: parameters in declaration order, then custom
    /// targets in registration order.
    pub fn mappings(&self) -> impl ExactSizeIterator<Item = (TargetId, &Mapping)> {
        self.mappings
            .iter()
            .map(|(&target, mapping)| (target, mapping))
    }

    /// Returns how many targets are mapped.
    pub fn len(&self) -> usize {
        self.mappings.len()
    }

    /// Returns whether no target is mapped.
    pub fn is_empty(&self) -> bool {
        self.mappings.is_empty()
    }

    fn known(&self, target: TargetId) -> bool {
        self.key(target).is_some()
    }

    /// The target registered or declared under `key`.
    fn target_for(&self, key: &str) -> Option<TargetId> {
        if let Some(id) = self.set.id(key) {
            return Some(TargetId::Param(id));
        }
        self.customs
            .iter()
            .find(|(_, custom)| custom.key == key)
            .map(|(&id, _)| TargetId::Custom(id))
    }

    fn check(&self, target: TargetId, mapping: &Mapping) -> Result<(), LearnError> {
        if !self.known(target) {
            return Err(LearnError::UnknownTarget(target));
        }
        let source = mapping.source;
        if !source.is_valid() {
            return Err(LearnError::BadSource(source));
        }
        let reserved = self
            .config
            .reserved_nrpns
            .iter()
            .any(|r| r.overlaps(&source));
        if source.control.parser_owned() || reserved {
            return Err(LearnError::Blocked(source));
        }
        let full = self.mappings.len() >= self.config.max_mappings;
        if full && !self.mappings.contains_key(&target) {
            return Err(LearnError::Full);
        }
        Ok(())
    }

    fn publish(&mut self) {
        let customs = &self.customs;
        let table = Table::new(self.mappings.iter().map(|(target, mapping)| {
            let route = match target {
                TargetId::Param(id) => Route::Param(*id),
                TargetId::Custom(id) => {
                    let staging = customs.get(id).and_then(|custom| custom.staging.as_ref());
                    Route::Staged(Arc::clone(
                        staging.expect("a mapped custom target is registered, with its staging"),
                    ))
                }
            };
            (route, mapping)
        }));
        self.publisher.publish(table);
    }
}

/// What capture refuses, from the config.
fn policy(config: &Config) -> Policy {
    Policy {
        channel_mode: config.reserve_channel_mode,
        ccs: config.reserved_ccs.clone(),
        nrpns: config.reserved_nrpns.clone(),
    }
}

impl Drop for Learn {
    /// Closes every open burst.
    fn drop(&mut self) {
        for custom in self.customs.values_mut() {
            custom.close();
        }
    }
}
