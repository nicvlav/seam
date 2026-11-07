//! What a parameter is, declared once.
//!
//! A declaration feeds every projection. The host formats read its name,
//! unit, range and step count. The page's TypeScript parameter map is
//! generated from it (`seam_bridge::typescript::param_map`). The saved state
//! keys its value by [`ParamDecl::key`].

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

/// A parameter's place in
/// [declaration order](crate::ParamSetBuilder::build), which a parameter set
/// mints and every read and write takes.
///
/// It is the order hosts list parameters in, not their automation identity:
/// hosts know a parameter by the [`host_id`](fn@crate::host_id) of its key, so
/// a later version can insert, reorder, regroup or remove parameters without
/// orphaning automation. The key is what must never change.
///
/// # Examples
///
/// ```
/// use seam_params::{Kind, ParamDecl, ParamSet};
///
/// const GAIN: ParamDecl =
///     ParamDecl::new("gain", "Gain", Kind::continuous(-24.0, 24.0, 0.1, None), 0.0)
///         .with_unit("dB");
/// let set = ParamSet::from_decls([GAIN])?;
///
/// // Look the id up once, by key, and keep it.
/// let gain = set.id("gain").ok_or("declared")?;
/// assert_eq!(set.value(gain), 0.0);
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ParamId(u32);

impl ParamId {
    /// The id at `index`. A set never holds 2^32 parameters, so saturating
    /// only keeps this total.
    pub(crate) fn from_index(index: usize) -> Self {
        ParamId(u32::try_from(index).unwrap_or(u32::MAX))
    }

    /// The position in declaration order.
    pub const fn index(self) -> usize {
        self.0 as usize
    }
}

/// What kind of value a parameter holds. Every kind stores its value as an
/// `f32` in real units.
///
/// # Examples
///
/// ```
/// use seam_params::Kind;
///
/// const ENABLED: Kind = Kind::Bool;
/// const MODE: Kind = Kind::Choice(&["Peak", "Shelf", "Notch"]);
///
/// assert_eq!(ENABLED.snap(0.7), Some(1.0));
/// assert_eq!(MODE.snap(1.4), Some(1.0)); // the index of "Shelf"
/// assert_eq!(MODE.end(), 2.0);
/// ```
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Kind {
    /// A number in a range, continuous or stepped.
    Continuous(Range),
    /// Off or on, stored as 0 or 1. A write of 0.5 or more is on.
    Bool,
    /// One of these labels, stored as its index. A write rounds to the
    /// nearest index, a tie to the even one, and clamps. Needs at least two
    /// labels.
    Choice(&'static [&'static str]),
}

impl Kind {
    /// A continuous or stepped range, for a declaration that is a constant:
    /// see [`Range::new`] for what the arguments mean.
    ///
    /// # Panics
    ///
    /// If [`Range::new`] would refuse the arguments. In a `const` that is a
    /// build error.
    ///
    /// # Examples
    ///
    /// ```
    /// use seam_params::Kind;
    ///
    /// // A gain from -24 dB to 24 dB in steps of 0.1 dB.
    /// const GAIN: Kind = Kind::continuous(-24.0, 24.0, 0.1, None);
    ///
    /// assert_eq!(GAIN.snap(3.04), Some(3.0));
    /// ```
    #[expect(
        clippy::panic,
        reason = "a declaration that is a constant fails the build; a refused range is a programming error, documented under Panics"
    )]
    pub const fn continuous(start: f32, end: f32, interval: f32, skew_centre: Option<f32>) -> Kind {
        match Range::new(start, end, interval, skew_centre) {
            Ok(range) => Kind::Continuous(range),
            Err(_) => panic!("Kind::continuous was given a range Range::new refuses"),
        }
    }

    /// The lowest legal value (never -0).
    pub const fn start(&self) -> f32 {
        match self {
            Kind::Continuous(range) => range.start + 0.0,
            Kind::Bool | Kind::Choice(_) => 0.0,
        }
    }

    /// The highest legal value: the range's end, 1 for a bool, or a choice's
    /// last index. [`Kind`] shows a choice's.
    pub fn end(&self) -> f32 {
        match self {
            Kind::Continuous(range) => range.end + 0.0,
            Kind::Bool => 1.0,
            Kind::Choice(labels) => last_index(labels),
        }
    }

    /// The legal value nearest `value`, in real units, or `None` when
    /// `value` is not finite, which each origin replaces with its own
    /// fallback.
    ///
    /// Snapping a snapped value changes nothing, and the result is never
    /// -0. A huge choice value clamps to the last index and never wraps.
    /// The kind must be one the parameter builder accepts.
    ///
    /// # Examples
    ///
    /// ```
    /// use seam_params::Kind;
    ///
    /// let gain = Kind::continuous(-24.0, 24.0, 0.1, None);
    /// assert_eq!(gain.snap(100.0), Some(24.0)); // clamped to the range
    /// assert_eq!(gain.snap(f32::NAN), None); // refused: the caller picks a fallback
    /// ```
    pub fn snap(&self, value: f32) -> Option<f32> {
        if !value.is_finite() {
            return None;
        }
        let snapped = match self {
            Kind::Continuous(range) => range.snap(value),
            Kind::Bool => {
                if value >= 0.5 {
                    1.0
                } else {
                    0.0
                }
            }
            Kind::Choice(labels) => value.round_ties_even().clamp(0.0, last_index(labels)),
        };
        // -0 + 0 is +0, so no store ever holds -0.
        Some(snapped + 0.0)
    }

    /// `value`, in real units, as a host's 0..1, clamped to it and not
    /// snapped.
    ///
    /// NaN maps to 0. Hosts carry 0..1 as a double, so it is an `f64`.
    ///
    /// # Examples
    ///
    /// ```
    /// use seam_params::Kind;
    ///
    /// // Skewed so 1 kHz sits at the middle of the knob.
    /// let cutoff = Kind::continuous(20.0, 20_000.0, 1.0, Some(1_000.0));
    /// assert!((cutoff.normalize(1_000.0) - 0.5).abs() < 1e-9);
    /// assert_eq!(cutoff.normalize(f32::NAN), 0.0);
    /// ```
    pub fn normalize(&self, value: f32) -> f64 {
        if value.is_nan() {
            return 0.0;
        }
        let normalized = match self {
            Kind::Continuous(range) => range.normalize(f64::from(value)),
            Kind::Bool => f64::from(value),
            Kind::Choice(labels) => f64::from(value) / choice_span(labels),
        };
        normalized.clamp(0.0, 1.0)
    }

    /// A host's 0..1 `normalized` in real units, before snapping. It clamps
    /// to 0..1 first, so an infinity lands on an end; NaN maps to the start.
    ///
    /// # Examples
    ///
    /// ```
    /// use seam_params::Kind;
    ///
    /// let gain = Kind::continuous(-24.0, 24.0, 0.1, None);
    /// assert_eq!(gain.denormalize(0.5), 0.0);
    /// assert_eq!(gain.denormalize(f64::INFINITY), 24.0); // clamped to the end
    /// assert_eq!(gain.denormalize(f64::NAN), -24.0); // the start
    ///
    /// // Not snapped: a write that takes it snaps afterwards.
    /// assert!((gain.denormalize(0.51) - 0.48).abs() < 1e-5);
    /// assert_eq!(gain.snap(gain.denormalize(0.51)), Some(0.5));
    /// ```
    pub fn denormalize(&self, normalized: f64) -> f32 {
        let normalized = if normalized.is_nan() {
            0.0
        } else {
            normalized.clamp(0.0, 1.0)
        };
        match self {
            Kind::Continuous(range) => range.denormalize(normalized),
            Kind::Bool => normalized as f32,
            Kind::Choice(labels) => (normalized * choice_span(labels)) as f32,
        }
    }
}

/// A choice's last index, as a value.
fn last_index(labels: &[&str]) -> f32 {
    labels.len().saturating_sub(1) as f32
}

/// The real-units width of a choice's 0..1, floored at 1 so a degenerate
/// choice cannot divide by zero.
fn choice_span(labels: &[&str]) -> f64 {
    labels.len().saturating_sub(1).max(1) as f64
}

/// A continuous parameter's range.
///
/// The legal values are the lattice points `start + k * interval` up to
/// `end`, plus `end` itself even when it is off the lattice. Every write,
/// from every origin, goes to the nearest legal value, a tie to the higher
/// one, so snapping twice changes nothing. Skew changes only how 0..1 maps
/// onto the range, never where values snap.
///
/// A stepped range counts in the finest decimal place it was written with. From
/// -60 in steps of 0.1 it counts in tenths, so every point, 0 dB included, is
/// the nearest `f32` to its decimal and not an `f32` step added up.
///
/// # Examples
///
/// ```
/// use seam_params::Range;
///
/// // A frequency from 20 Hz to 20 kHz in steps of 1 Hz, with 1 kHz at the
/// // middle of the knob.
/// let hertz = Range::new(20.0, 20_000.0, 1.0, Some(1_000.0))?;
/// assert_eq!(hertz.end(), 20_000.0);
///
/// assert!(Range::new(5.0, 5.0, 0.0, None).is_err());
/// # Ok::<(), seam_params::RangeError>(())
/// ```
#[derive(Clone, Copy, PartialEq)]
pub struct Range {
    start: f32,
    end: f32,
    interval: f32,
    skew_centre: Option<f32>,
    /// Laid out once, here, since every host and learned write snaps on the
    /// audio thread and laying it out takes about as long again as the rest
    /// of the write.
    lattice: Option<Lattice>,
}

impl std::fmt::Debug for Range {
    /// The range as declared, without the lattice laid out from it.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Range")
            .field("start", &self.start)
            .field("end", &self.end)
            .field("interval", &self.interval)
            .field("skew_centre", &self.skew_centre)
            .finish_non_exhaustive()
    }
}

/// Why a [`Range`] was refused.
///
/// # Examples
///
/// ```
/// use seam_params::{Range, RangeError};
///
/// assert_eq!(Range::new(5.0, 1.0, 0.0, None), Err(RangeError::StartNotBelowEnd));
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum RangeError {
    /// The start or the end is not finite.
    NotFinite,
    /// The start is not below the end.
    StartNotBelowEnd,
    /// The interval is not finite, or is negative.
    BadInterval,
    /// The skew centre is not strictly inside the range.
    SkewCentreOutside,
}

impl std::fmt::Display for RangeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            RangeError::NotFinite => "start and end must be finite",
            RangeError::StartNotBelowEnd => "start must be below end",
            RangeError::BadInterval => "the interval must be finite and not negative",
            RangeError::SkewCentreOutside => "the skew centre must be strictly inside the range",
        })
    }
}

impl std::error::Error for RangeError {}

impl Range {
    /// A range from `start` to `end`, snapping to every `interval` from the
    /// start (0 for none), and with `skew_centre` at the middle of 0..1
    /// (`None` for a linear knob).
    ///
    /// # Errors
    ///
    /// Returns the [`RangeError`] naming the first rule the arguments break;
    /// its variants are the rules.
    pub const fn new(
        start: f32,
        end: f32,
        interval: f32,
        skew_centre: Option<f32>,
    ) -> Result<Range, RangeError> {
        if !start.is_finite() || !end.is_finite() {
            return Err(RangeError::NotFinite);
        }
        if start >= end {
            return Err(RangeError::StartNotBelowEnd);
        }
        if !interval.is_finite() || interval < 0.0 {
            return Err(RangeError::BadInterval);
        }
        if let Some(c) = skew_centre
            && (!c.is_finite() || c <= start || c >= end)
        {
            return Err(RangeError::SkewCentreOutside);
        }
        // -0 compares equal to 0, so the lowest value is stored as 0 and never
        // shows a sign.
        let start = if start == 0.0 { 0.0 } else { start };
        Ok(Range {
            start,
            end,
            interval,
            skew_centre,
            lattice: Lattice::of(start, end, interval),
        })
    }

    /// The lowest legal value (never -0).
    pub const fn start(&self) -> f32 {
        self.start
    }

    /// The highest value.
    pub const fn end(&self) -> f32 {
        self.end
    }

    /// The snapping step; 0 means continuous.
    pub const fn interval(&self) -> f32 {
        self.interval
    }

    /// The value that sits at the middle of 0..1, for a skewed range.
    pub const fn skew_centre(&self) -> Option<f32> {
        self.skew_centre
    }

    /// The lattice a stepped range snaps to; `None` for a continuous one.
    pub(crate) const fn lattice(&self) -> Option<Lattice> {
        self.lattice
    }
}

/// The decimal an `f32` was written as, with its places. It is the one with
/// the fewest places, up to nine, that reads back as the same `f32`.
/// Otherwise it is the `f32` itself and `None`.
#[expect(clippy::arithmetic_side_effects, reason = "the count stops at ten")]
pub(crate) const fn shortest_decimal(value: f32) -> (f64, Option<u32>) {
    let wide = value as f64;
    let mut places = 0;
    while places < 10 {
        let scale = ten_to(places);
        let decimal = (wide * scale).round() / scale;
        if decimal as f32 == value {
            return (decimal, Some(places));
        }
        places += 1;
    }
    (wide, None)
}

/// 10 to the power `places`, exactly for every count `shortest_decimal` uses.
#[expect(
    clippy::arithmetic_side_effects,
    reason = "the count stops at `places`"
)]
const fn ten_to(places: u32) -> f64 {
    let mut scale = 1.0;
    let mut done = 0;
    while done < places {
        scale *= 10.0;
        done += 1;
    }
    scale
}

/// Every whole number below this is exact in `f64`.
const EXACT: f64 = 9_007_199_254_740_992.0;

/// A stepped range's lattice: the points `(s + k * t) / scale` for `k` from 0
/// to `last`, which snapping and the host's step count both read.
///
/// Here `s` and `t` are the start and the interval in whole units of
/// `1 / scale`. A start or step past nine places, or ends too large to count
/// exactly in `f64`, falls back to a scale of 1, with `s` and `t` the
/// [`shortest_decimal`]s of the start and the step.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Lattice {
    s: f64,
    t: f64,
    scale: f64,
    /// The last point's index.
    pub(crate) last: f64,
    start: f32,
    end: f32,
}

impl Lattice {
    /// The lattice of a range from `start` to `end` stepping by `interval`;
    /// `None` for a continuous one, whose interval is 0.
    const fn of(start: f32, end: f32, interval: f32) -> Option<Lattice> {
        if interval.is_nan() || interval <= 0.0 {
            return None;
        }
        let (first, start_places) = shortest_decimal(start);
        let (step, step_places) = shortest_decimal(interval);
        let (last_point, _) = shortest_decimal(end);
        let mut lattice = (first, step, 1.0, last_point);
        if let (Some(a), Some(b)) = (start_places, step_places) {
            let scale = ten_to(if a > b { a } else { b });
            let (s, t, e) = (
                (first * scale).round(),
                (step * scale).round(),
                last_point * scale,
            );
            if t > 0.0 && s.abs() < EXACT && e.abs() < EXACT {
                lattice = (s, t, scale, e);
            }
        }
        let (s, t, scale, e) = lattice;
        Some(Lattice {
            s,
            t,
            scale,
            last: ((e - s) / t).floor().max(0.0),
            start,
            end,
        })
    }

    /// The index of the point at or below `value`, from 0 to the last.
    pub(crate) fn below(&self, value: f64) -> f64 {
        ((value * self.scale - self.s) / self.t)
            .floor()
            .clamp(0.0, self.last)
    }

    /// Point `k`, as the `f32` the range stores.
    pub(crate) fn point(&self, k: f64) -> f32 {
        (((self.s + k * self.t) / self.scale) as f32).clamp(self.start, self.end)
    }
}

impl Range {
    /// The nearest legal value to a finite `value`.
    fn snap(&self, value: f32) -> f32 {
        let (start, end) = (self.start, self.end);
        if value <= start {
            return start;
        }
        if value >= end {
            return end;
        }
        let Some(lattice) = self.lattice else {
            return value;
        };
        let v = f64::from(value);
        let k = lattice.below(v);
        let below = lattice.point(k);
        // Past the last lattice point, the end itself is the next legal value.
        let above = if k < lattice.last {
            lattice.point(k + 1.0)
        } else {
            end
        };
        // The candidates are f32s, so snapping one of them finds it again at
        // distance 0: that is what makes snapping idempotent.
        if (f64::from(above) - v).abs() <= (v - f64::from(below)).abs() {
            above
        } else {
            below
        }
    }

    /// The exponent that puts the skew centre at 0.5, if there is one.
    fn skew_exponent(&self) -> Option<f64> {
        let centre = f64::from(self.skew_centre?);
        let (s, e) = (f64::from(self.start), f64::from(self.end));
        let proportion = (centre - s) / (e - s);
        (proportion > 0.0 && proportion < 1.0).then(|| 0.5_f64.ln() / proportion.ln())
    }

    fn normalize(&self, value: f64) -> f64 {
        let (s, e) = (f64::from(self.start), f64::from(self.end));
        let span = e - s;
        if span.is_nan() || span <= 0.0 {
            return 0.0;
        }
        let proportion = ((value - s) / span).clamp(0.0, 1.0);
        match self.skew_exponent() {
            Some(exponent) => proportion.powf(exponent),
            None => proportion,
        }
    }

    /// `normalized` is in 0..1.
    fn denormalize(&self, normalized: f64) -> f32 {
        let (s, e) = (f64::from(self.start), f64::from(self.end));
        let proportion = match self.skew_exponent() {
            Some(exponent) => normalized.powf(exponent.recip()),
            None => normalized,
        };
        (s + (e - s) * proportion) as f32
    }
}

/// One parameter, as its plugin declares it.
///
/// A plugin declares its parameters as constants and adds them to a
/// [`ParamSetBuilder`](crate::ParamSetBuilder), which checks them once. The
/// methods below say what each origin's write stores.
///
/// # Examples
///
/// ```
/// use seam_params::{Kind, ParamDecl};
///
/// const GAIN: ParamDecl = ParamDecl::new(
///     "low.gain",
///     "Low gain",
///     Kind::continuous(-24.0, 24.0, 0.1, None),
///     0.0,
/// )
///     .with_unit("dB")
///     .with_group("Low");
///
/// assert_eq!(GAIN.default_value(), 0.0);
/// ```
#[derive(Debug, Clone, Copy, PartialEq)]
#[non_exhaustive]
pub struct ParamDecl {
    /// The stable identity: saved states key the value by it and the page
    /// names the parameter by it.
    ///
    /// Never the display name, so a rename keeps sessions. The builder
    /// refuses a key that is empty, has a `/`, repeats another or is longer
    /// than [`MAX_KEY_LEN`](crate::MAX_KEY_LEN) bytes.
    pub key: &'static str,
    /// The name a host and the page show.
    pub name: &'static str,
    /// The unit shown after the value ("dB", "%"); empty for none.
    pub unit: &'static str,
    /// The group path, `/`-separated ("Mix/Sides"); empty for the top level.
    /// Presentation only: lookup is by key, so moving a parameter between
    /// groups keeps sessions.
    pub group: &'static str,
    /// What it holds.
    pub kind: Kind,
    /// The value it starts at and returns to, in real units; snapped like any
    /// write.
    pub default: f32,
    /// Whether it is the plugin's bypass, which the host may drive as such.
    /// At most one parameter is, and it is a bool.
    pub bypass: bool,
}

impl ParamDecl {
    /// A declaration with the four things every parameter has: its key, its
    /// display name, what it holds and where it starts.
    ///
    /// It has no unit, sits at the top level and is not the bypass; the
    /// `with_` methods and [`as_bypass`](Self::as_bypass) set those. Outside
    /// this crate these are the only way to make one. [`ParamDecl`] shows one
    /// built and read.
    pub const fn new(key: &'static str, name: &'static str, kind: Kind, default: f32) -> ParamDecl {
        ParamDecl {
            key,
            name,
            unit: "",
            group: "",
            kind,
            default,
            bypass: false,
        }
    }

    /// Sets the unit shown after the value ("dB", "%"). [`ParamDecl`] shows
    /// it.
    #[must_use]
    pub const fn with_unit(mut self, unit: &'static str) -> ParamDecl {
        self.unit = unit;
        self
    }

    /// Puts it in a group: a `/`-separated path ("Mix/Sides"). [`ParamDecl`]
    /// shows it, and [`ParamSetBuilder::build`](crate::ParamSetBuilder::build)
    /// how groups order the set.
    #[must_use]
    pub const fn with_group(mut self, group: &'static str) -> ParamDecl {
        self.group = group;
        self
    }

    /// Makes it the plugin's [`bypass`](Self::bypass).
    /// [`ParamSetBuilder`](crate::ParamSetBuilder) shows one declared.
    #[must_use]
    pub const fn as_bypass(mut self) -> ParamDecl {
        self.bypass = true;
        self
    }
}

/// What each origin's write stores, before the store compares it with the
/// current value. Every result is a legal value; where a write receives a
/// value that is not finite, each origin falls back as its method says.
impl ParamDecl {
    /// The value the store starts at and a reset returns to: the declared
    /// default, snapped, or the range start when it is not finite.
    ///
    /// # Examples
    ///
    /// ```
    /// use seam_params::{Kind, ParamDecl};
    ///
    /// let kind = Kind::continuous(-1.0, 1.0, 0.5, None);
    ///
    /// // Off the 0.5 lattice: the default snaps to it.
    /// assert_eq!(ParamDecl::new("a", "A", kind, 0.3).default_value(), 0.5);
    /// // Not finite: the range start.
    /// assert_eq!(ParamDecl::new("a", "A", kind, f32::NAN).default_value(), -1.0);
    /// ```
    pub fn default_value(&self) -> f32 {
        self.kind.snap(self.default).unwrap_or(self.kind.start())
    }

    /// What a host write of the 0..1 `normalized` stores. A value that
    /// is not finite becomes 0, the range start. Then it clamps to 0..1,
    /// converts to real units and snaps.
    ///
    /// # Examples
    ///
    /// ```
    /// use seam_params::{Kind, ParamDecl};
    ///
    /// const GAIN: ParamDecl =
    ///     ParamDecl::new("gain", "Gain", Kind::continuous(-24.0, 24.0, 0.1, None), 6.0);
    ///
    /// assert_eq!(GAIN.host_value(0.5), 0.0);
    /// assert_eq!(GAIN.host_value(7.0), 24.0); // clamped to 0..1
    /// assert_eq!(GAIN.host_value(f64::NAN), -24.0); // the range start
    /// ```
    pub fn host_value(&self, normalized: f64) -> f32 {
        let normalized = if normalized.is_finite() {
            normalized
        } else {
            0.0
        };
        let real = self.kind.denormalize(normalized);
        self.kind.snap(real).unwrap_or(self.kind.start())
    }

    /// What a restore of `value`, in real units, stores: snapped, or
    /// the default when it is not finite.
    ///
    /// # Examples
    ///
    /// ```
    /// use seam_params::{Kind, ParamDecl};
    ///
    /// const GAIN: ParamDecl =
    ///     ParamDecl::new("gain", "Gain", Kind::continuous(-24.0, 24.0, 0.1, None), 6.0);
    ///
    /// assert_eq!(GAIN.restore_value(99.0), 24.0); // snapped to the range
    /// assert_eq!(GAIN.restore_value(f32::NAN), 6.0); // the declared default
    /// ```
    pub fn restore_value(&self, value: f32) -> f32 {
        self.kind
            .snap(value)
            .unwrap_or_else(|| self.default_value())
    }

    /// What a learned controller's 0..1 `normalized` stores, or `None`
    /// to keep the current value, for NaN. An infinity lands on an end,
    /// since the value clamps to 0..1 before it converts.
    ///
    /// # Examples
    ///
    /// ```
    /// use seam_params::{Kind, ParamDecl};
    ///
    /// const GAIN: ParamDecl =
    ///     ParamDecl::new("gain", "Gain", Kind::continuous(-24.0, 24.0, 0.1, None), 6.0);
    ///
    /// assert_eq!(GAIN.learned_value(0.5), Some(0.0));
    /// assert_eq!(GAIN.learned_value(f64::INFINITY), Some(24.0)); // an end
    /// assert_eq!(GAIN.learned_value(f64::NAN), None); // keep the current value
    /// ```
    pub fn learned_value(&self, normalized: f64) -> Option<f32> {
        if normalized.is_nan() {
            return None;
        }
        self.kind.snap(self.kind.denormalize(normalized))
    }

    /// What a user or program write of `value`, in real units, stores, or
    /// `None` when it is refused because it is not finite: nothing is
    /// stored and nothing is told.
    ///
    /// # Examples
    ///
    /// ```
    /// use seam_params::{Kind, ParamDecl};
    ///
    /// const GAIN: ParamDecl =
    ///     ParamDecl::new("gain", "Gain", Kind::continuous(-24.0, 24.0, 0.1, None), 6.0);
    ///
    /// assert_eq!(GAIN.edit_value(-3.04), Some(-3.0)); // snapped
    /// assert_eq!(GAIN.edit_value(f32::NAN), None); // refused
    /// ```
    pub fn edit_value(&self, value: f32) -> Option<f32> {
        self.kind.snap(value)
    }
}

#[cfg(test)]
mod tests;
