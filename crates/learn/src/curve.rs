//! A mapping's curve, from a controller's 0 to 1 to the target's 0 to 1.

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

/// What a mapping does to a controller's value before the target's range:
/// clamp to 0..1, invert, skew, then map into a window. Absolute only, no
/// pickup or takeover.
///
/// Checked when it is made ([`Curve::new`]), so every curve maps 0..1 into
/// 0..1. The identity, the default, returns every value in 0..1 unchanged,
/// bit for bit.
///
/// # Examples
///
/// ```
/// use seam_learn::Curve;
///
/// // A knob that should be fine at the low end: skew 2 squares the value.
/// let fine_low = Curve::new(2.0, 0.0, 1.0)?;
/// assert_eq!(fine_low.apply(0.5), 0.25);
/// assert!(!fine_low.is_identity());
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Curve {
    invert: bool,
    skew: f32,
    low: f32,
    high: f32,
}

/// Why a curve was refused.
///
/// # Examples
///
/// ```
/// use seam_learn::{Curve, CurveError};
///
/// // The ends of a window must lie inside 0..1.
/// assert_eq!(Curve::new(1.0, 0.0, 1.5), Err(CurveError::Window));
/// assert_eq!(Curve::new(f32::NAN, 0.0, 1.0), Err(CurveError::Skew));
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum CurveError {
    /// The skew is not a finite number above 0.
    Skew,
    /// An end of the window is not a finite number in 0..1.
    Window,
}

impl fmt::Display for CurveError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CurveError::Skew => write!(f, "a curve's skew must be finite and above 0"),
            CurveError::Window => write!(f, "a curve's window must lie inside 0..1"),
        }
    }
}

impl std::error::Error for CurveError {}

impl Curve {
    /// The curve that changes nothing.
    pub const IDENTITY: Curve = Curve {
        invert: false,
        skew: 1.0,
        low: 0.0,
        high: 1.0,
    };

    /// Creates a curve that raises to `skew`, then maps 0 to `low` and 1 to
    /// `high`.
    ///
    /// A `low` above `high` is allowed and runs the window backwards, which is
    /// not an inversion of the input; see [`inverted`](Curve::inverted). The
    /// [`Curve`] example makes one.
    ///
    /// # Errors
    ///
    /// Returns [`CurveError::Skew`] when `skew` is not finite and above 0,
    /// and [`CurveError::Window`] when `low` or `high` is outside 0..1.
    pub fn new(skew: f32, low: f32, high: f32) -> Result<Curve, CurveError> {
        if !(skew.is_finite() && skew > 0.0) {
            return Err(CurveError::Skew);
        }
        if ![low, high].iter().all(|end| (0.0..=1.0).contains(end)) {
            return Err(CurveError::Window);
        }
        // `+ 0.0` turns a -0 end into 0, so a saved curve never writes -0.
        Ok(Curve {
            invert: false,
            skew,
            low: low + 0.0,
            high: high + 0.0,
        })
    }

    /// The same curve, inverting its input before it skews.
    ///
    /// # Examples
    ///
    /// ```
    /// use seam_learn::Curve;
    ///
    /// assert_eq!(Curve::IDENTITY.inverted().apply(1.0), 0.0);
    /// assert!(Curve::IDENTITY.inverted().is_inverted());
    /// ```
    #[must_use]
    pub const fn inverted(self) -> Curve {
        Curve {
            invert: true,
            ..self
        }
    }

    /// Returns whether it inverts.
    pub fn is_inverted(&self) -> bool {
        self.invert
    }

    /// Returns the exponent applied after inverting; 1 is linear.
    pub fn skew(&self) -> f32 {
        self.skew
    }

    /// Returns the window's low end, which the skewed value 0 maps to.
    pub fn low(&self) -> f32 {
        self.low
    }

    /// Returns the window's high end, which the skewed value 1 maps to.
    pub fn high(&self) -> f32 {
        self.high
    }

    /// Returns whether it changes nothing: it equals [`Curve::IDENTITY`].
    pub fn is_identity(&self) -> bool {
        *self == Curve::IDENTITY
    }

    /// The curve applied to a controller's 0..1: clamp, invert, skew,
    /// window. NaN stays NaN, which a learned write ignores.
    ///
    /// # Examples
    ///
    /// ```
    /// use seam_learn::Curve;
    ///
    /// let curve = Curve::new(1.0, 0.0, 0.5)?.inverted();
    ///
    /// // Clamped, inverted, then mapped into the window 0 to 0.5.
    /// assert_eq!(curve.apply(1.0), 0.0);
    /// assert_eq!(curve.apply(0.0), 0.5);
    /// assert_eq!(curve.apply(2.0), 0.0, "past 1 clamps first");
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    pub fn apply(&self, value: f64) -> f64 {
        let mut value = value.clamp(0.0, 1.0);
        if self.invert {
            value = 1.0 - value;
        }
        if self.skew != 1.0 {
            value = value.powf(f64::from(self.skew));
        }
        let (low, high) = (f64::from(self.low), f64::from(self.high));
        low + value * (high - low)
    }
}

impl Default for Curve {
    fn default() -> Self {
        Curve::IDENTITY
    }
}
