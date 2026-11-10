//! Typed handles: a parameter as plugin code holds it, bound to its
//! declaration once and read as the type its kind stands for.

// Audio-path code: the audio thread decodes through these handles, and a
// panic here ends a host, so indexing, unwrap, expect, panic and unchecked
// arithmetic are refused.
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
use std::marker::PhantomData;

use crate::{Kind, ParamId, ParamSet};

/// A parameter as plugin code holds it: an id that knows what its value reads
/// as.
///
/// Every read and write that takes a parameter takes a handle:
/// [`ParamSet::value`], [`AudioParams::value`](crate::AudioParams::value),
/// [`UiParams::value`](crate::UiParams::value) and
/// [`UiParams::write`](crate::UiParams::write). [`ParamId`] is the untyped
/// handle, whose value is the stored `f32`. [`Float`], [`Bool`] and
/// [`Choice`] are the typed ones, which [`ParamSet::bind`] makes after
/// checking the declaration, so a read never has to check again.
///
/// # Examples
///
/// ```
/// use seam_params::{Bool, Handle, Kind, ParamDecl, ParamSet};
///
/// let set = ParamSet::from_decls([ParamDecl::new("on", "On", Kind::Bool, 1.0)])?;
/// let on: Bool = set.bind("on")?;
///
/// assert!(set.value(on)); // a bool, not 1.0
/// assert_eq!(set.value(on.id()), 1.0); // the stored value, through its id
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
pub trait Handle: Copy {
    /// What a read returns and a write takes: `f32`, `bool` or a plugin's
    /// enum.
    type Value;

    /// Returns the parameter it names.
    fn id(self) -> ParamId;

    /// Returns a stored value, in real units, as this handle reads it.
    ///
    /// It must give a value for any `f32`, NaN and the infinities included,
    /// and never panic: the audio thread calls it on every read.
    fn decode(self, stored: f32) -> Self::Value;

    /// Returns `value` as the `f32` a write takes, in real units, which the
    /// write then snaps.
    fn encode(self, value: Self::Value) -> f32;
}

impl Handle for ParamId {
    type Value = f32;

    fn id(self) -> ParamId {
        self
    }

    fn decode(self, stored: f32) -> f32 {
        stored
    }

    fn encode(self, value: f32) -> f32 {
        value
    }
}

/// A continuous parameter, read as its value in real units.
///
/// # Examples
///
/// ```
/// use seam_params::{Float, Kind, ParamDecl, ParamSet};
///
/// let set = ParamSet::from_decls([
///     ParamDecl::new("gain", "Gain", Kind::continuous(-60.0, 12.0, 0.1, None), -6.0),
/// ])?;
/// let gain: Float = set.bind("gain")?;
///
/// assert_eq!(set.value(gain), -6.0); // in dB, as declared
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Float(ParamId);

/// A bool parameter, read as `bool`.
///
/// # Examples
///
/// ```
/// use seam_params::{Bool, Kind, ParamDecl, ParamSet};
///
/// let set = ParamSet::from_decls([ParamDecl::new("on", "On", Kind::Bool, 0.0)])?;
/// let on: Bool = set.bind("on")?;
/// let ui = set.take_ui().ok_or("the first UI handle")?;
///
/// assert!(ui.write(on, true));
/// assert!(set.audio().value(on));
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Bool(ParamId);

/// A choice parameter, read as the plugin's enum `E`, whose
/// [`Choices::LABELS`] are the declaration's labels.
///
/// It is `Copy`, `Send` and `Sync` whatever `E` is, since it holds no `E`.
///
/// # Examples
///
/// ```
/// use seam_params::{Choice, Choices, Kind, ParamDecl, ParamSet};
///
/// #[derive(Debug, Clone, Copy, PartialEq)]
/// enum Shape {
///     Sine,
///     Saw,
/// }
///
/// impl Choices for Shape {
///     const LABELS: &'static [&'static str] = &["Sine", "Saw"];
///
///     fn from_index(index: usize) -> Self {
///         match index {
///             0 => Shape::Sine,
///             _ => Shape::Saw,
///         }
///     }
///
///     fn index(self) -> usize {
///         self as usize
///     }
/// }
///
/// let set = ParamSet::from_decls([
///     ParamDecl::new("shape", "Shape", Kind::Choice(Shape::LABELS), 1.0),
/// ])?;
/// let shape: Choice<Shape> = set.bind("shape")?;
///
/// assert_eq!(set.value(shape), Shape::Saw);
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
pub struct Choice<E>(ParamId, PhantomData<fn() -> E>);

// Written by hand: derives would bound each on `E`, which the handle never
// holds.
impl<E> Clone for Choice<E> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<E> Copy for Choice<E> {}

impl<E> PartialEq for Choice<E> {
    fn eq(&self, other: &Self) -> bool {
        self.0 == other.0
    }
}

impl<E> Eq for Choice<E> {}

impl<E> std::hash::Hash for Choice<E> {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.0.hash(state);
    }
}

impl<E> fmt::Debug for Choice<E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("Choice").field(&self.0).finish()
    }
}

/// An enum that names a choice's values, in label order.
///
/// The declaration takes its labels from here, `Kind::Choice(E::LABELS)`, so
/// the enum and the parameter cannot drift apart; [`ParamSet::bind`] refuses
/// a [`Choice<E>`] whose labels differ all the same.
///
/// # Examples
///
/// ```
/// use seam_params::{Choices, Kind, ParamDecl};
///
/// #[derive(Debug, Clone, Copy, PartialEq)]
/// enum Slope {
///     Gentle,
///     Steep,
/// }
///
/// impl Choices for Slope {
///     const LABELS: &'static [&'static str] = &["12 dB/oct", "24 dB/oct"];
///
///     fn from_index(index: usize) -> Self {
///         match index {
///             0 => Slope::Gentle,
///             _ => Slope::Steep,
///         }
///     }
///
///     fn index(self) -> usize {
///         self as usize
///     }
/// }
///
/// // The declaration takes the enum's labels, so the two are written once.
/// const SLOPE: ParamDecl = ParamDecl::new("slope", "Slope", Kind::Choice(Slope::LABELS), 0.0);
/// assert_eq!(SLOPE.kind, Kind::Choice(&["12 dB/oct", "24 dB/oct"]));
/// assert_eq!(Slope::from_index(usize::MAX), Slope::Steep); // total
/// ```
pub trait Choices: Copy + 'static {
    /// The labels hosts show, one per value, in index order.
    const LABELS: &'static [&'static str];

    /// Returns the value at `index`.
    ///
    /// It must give a value for any `usize` and never panic: the audio thread
    /// calls it on every read. A bound handle only ever passes an index below
    /// the labels' count, so the value past it is the plugin's choice; the
    /// last is the usual one.
    fn from_index(index: usize) -> Self;

    /// Returns the value's index into [`LABELS`](Self::LABELS).
    fn index(self) -> usize;
}

impl Handle for Float {
    type Value = f32;

    fn id(self) -> ParamId {
        self.0
    }

    fn decode(self, stored: f32) -> f32 {
        stored
    }

    fn encode(self, value: f32) -> f32 {
        value
    }
}

impl Handle for Bool {
    type Value = bool;

    fn id(self) -> ParamId {
        self.0
    }

    /// On at 0.5 and above, as a bool's write snaps; NaN is off.
    fn decode(self, stored: f32) -> bool {
        stored >= 0.5
    }

    fn encode(self, value: bool) -> f32 {
        if value { 1.0 } else { 0.0 }
    }
}

impl<E: Choices> Handle for Choice<E> {
    type Value = E;

    fn id(self) -> ParamId {
        self.0
    }

    /// The value at the stored index. A choice stores whole indices; any
    /// other `f32` truncates, saturating at both ends, and NaN reads as 0.
    fn decode(self, stored: f32) -> E {
        // `as` from a float saturates and maps NaN to 0, so no index panics.
        E::from_index(stored as usize)
    }

    fn encode(self, value: E) -> f32 {
        value.index() as f32
    }
}

/// A handle [`ParamSet::bind`] can make from a key, checking the declaration
/// first.
///
/// [`Float`], [`Bool`] and [`Choice`] implement it. A plugin's own handle,
/// say a gain read as a factor, implements it by binding one of them.
///
/// # Examples
///
/// ```
/// use seam_params::{Bind, BindError, Float, Handle, Kind, ParamDecl, ParamId, ParamSet};
///
/// /// A gain in dB, read as the factor a block multiplies by.
/// #[derive(Clone, Copy)]
/// struct Factor(Float);
///
/// impl Handle for Factor {
///     type Value = f32;
///     fn id(self) -> ParamId {
///         self.0.id()
///     }
///     fn decode(self, stored: f32) -> f32 {
///         10.0_f32.powf(stored / 20.0)
///     }
///     fn encode(self, factor: f32) -> f32 {
///         20.0 * factor.log10()
///     }
/// }
///
/// impl Bind for Factor {
///     fn bind(set: &ParamSet, key: &'static str) -> Result<Self, BindError> {
///         Float::bind(set, key).map(Factor)
///     }
/// }
///
/// let set = ParamSet::from_decls([
///     ParamDecl::new("gain", "Gain", Kind::continuous(-60.0, 12.0, 0.0, None), -20.0),
/// ])?;
/// let gain: Factor = set.bind("gain")?;
/// assert!((set.value(gain) - 0.1).abs() < 1e-6);
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
pub trait Bind: Handle + Sized {
    /// Returns the handle for the parameter declared under `key` in `set`.
    ///
    /// # Errors
    ///
    /// Returns the [`BindError`] that says why the declaration does not
    /// match the handle.
    fn bind(set: &ParamSet, key: &'static str) -> Result<Self, BindError>;
}

/// The id declared under `key`, and its kind.
fn declared(set: &ParamSet, key: &'static str) -> Result<(ParamId, Kind), BindError> {
    let id = set.id(key).ok_or(BindError::NotDeclared(key))?;
    Ok((id, set.decl(id).kind))
}

impl Bind for Float {
    fn bind(set: &ParamSet, key: &'static str) -> Result<Self, BindError> {
        match declared(set, key)? {
            (id, Kind::Continuous(_)) => Ok(Float(id)),
            _ => Err(BindError::WrongKind(key)),
        }
    }
}

impl Bind for Bool {
    fn bind(set: &ParamSet, key: &'static str) -> Result<Self, BindError> {
        match declared(set, key)? {
            (id, Kind::Bool) => Ok(Bool(id)),
            _ => Err(BindError::WrongKind(key)),
        }
    }
}

impl<E: Choices> Bind for Choice<E> {
    fn bind(set: &ParamSet, key: &'static str) -> Result<Self, BindError> {
        match declared(set, key)? {
            (id, Kind::Choice(labels)) if labels == E::LABELS => Ok(Choice(id, PhantomData)),
            (_, Kind::Choice(_)) => Err(BindError::LabelsDiffer(key)),
            _ => Err(BindError::WrongKind(key)),
        }
    }
}

/// Why a handle could not be bound: a mistake in the plugin's own
/// declarations or handles, found when the plugin is built, never in a block.
///
/// # Examples
///
/// ```
/// use seam_params::{BindError, Bool, Kind, ParamDecl, ParamSet};
///
/// let set = ParamSet::from_decls([
///     ParamDecl::new("gain", "Gain", Kind::continuous(-60.0, 12.0, 0.1, None), 0.0),
/// ])?;
///
/// assert_eq!(set.bind::<Bool>("gain"), Err(BindError::WrongKind("gain")));
/// assert_eq!(set.bind::<Bool>("bypass"), Err(BindError::NotDeclared("bypass")));
/// # Ok::<(), seam_params::DeclError>(())
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum BindError {
    /// No parameter is declared under the key.
    NotDeclared(&'static str),
    /// The parameter is declared as another kind than the handle reads.
    WrongKind(&'static str),
    /// The choice's labels are not the enum's [`Choices::LABELS`].
    LabelsDiffer(&'static str),
}

impl fmt::Display for BindError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            BindError::NotDeclared(key) => write!(f, "no parameter is declared as {key:?}"),
            BindError::WrongKind(key) => {
                write!(
                    f,
                    "{key:?} is declared as another kind than the handle reads"
                )
            }
            BindError::LabelsDiffer(key) => {
                write!(f, "the labels of choice {key:?} are not the enum's")
            }
        }
    }
}

impl std::error::Error for BindError {}

impl ParamSet {
    /// Binds a handle to the parameter declared under `key`, checking that
    /// its kind is the handle's, and for a [`Choice`] that its labels are the
    /// enum's. Nothing is checked after this: bind once, when the plugin is
    /// built, and keep the handle.
    ///
    /// # Errors
    ///
    /// Returns the [`BindError`] that names the key.
    ///
    /// # Examples
    ///
    /// A plugin binds a struct of handles in one function, each field's type
    /// saying which handle to make.
    ///
    /// ```
    /// use seam_params::{BindError, Bool, Float, Kind, ParamDecl, ParamSet};
    ///
    /// struct Params {
    ///     gain: Float,
    ///     bypass: Bool,
    /// }
    ///
    /// impl Params {
    ///     fn bind(set: &ParamSet) -> Result<Self, BindError> {
    ///         Ok(Params {
    ///             gain: set.bind("gain")?,
    ///             bypass: set.bind("bypass")?,
    ///         })
    ///     }
    /// }
    ///
    /// let set = ParamSet::from_decls([
    ///     ParamDecl::new("gain", "Gain", Kind::continuous(-60.0, 12.0, 0.1, None), 0.0),
    ///     ParamDecl::new("bypass", "Bypass", Kind::Bool, 0.0).as_bypass(),
    /// ])?;
    /// let params = Params::bind(&set)?;
    /// assert!(!set.value(params.bypass));
    /// assert_eq!(set.value(params.gain), 0.0);
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    pub fn bind<H: Bind>(&self, key: &'static str) -> Result<H, BindError> {
        H::bind(self, key)
    }
}
