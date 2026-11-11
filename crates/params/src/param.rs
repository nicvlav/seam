//! Typed parameters: a parameter as plugin code holds it, carrying its store
//! and read as the type its kind stands for.

// Audio-path code: the audio thread reads through these, and a panic here
// ends a host, so indexing, unwrap, expect, panic and unchecked arithmetic
// are refused.
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
use std::hash::{Hash, Hasher};
use std::marker::PhantomData;
use std::sync::Arc;

use crate::set::ParamCell;
use crate::{Kind, ParamId, ParamSet, UiParams, audio_thread};

/// A continuous parameter, read as its value in real units.
///
/// It carries its store: [`get`](Self::get) reads it with no set at hand,
/// from any thread. A clone is the same parameter, and two compare equal
/// only when they are.
///
/// # Panics
///
/// In a build with `debug_assertions`, dropping one panics while the thread
/// holds an [`audio_thread`] mark: the last drop frees the store. A clone is
/// allowed there.
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
/// assert_eq!(gain.get(), -6.0); // in dB, as declared
/// set.audio().host_write(gain.id(), 1.0); // the host moves it to the top
/// assert_eq!(gain.get(), 12.0);
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
pub struct Float(Arc<ParamCell>);

/// A bool parameter, read as `bool`.
///
/// It carries its store as [`Float`] does, and is cloned and dropped as one
/// is.
///
/// # Panics
///
/// In a build with `debug_assertions`, dropping one panics while the thread
/// holds an [`audio_thread`] mark.
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
/// assert!(ui.write(&on, true));
/// assert!(on.get());
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
pub struct Bool(Arc<ParamCell>);

/// A choice parameter, read as the plugin's enum `E`, whose
/// [`Choices::LABELS`] are the declaration's labels.
///
/// It carries its store as [`Float`] does, and is cloned and dropped as one
/// is. It is `Send` and `Sync` whatever `E` is, since it holds no `E`.
///
/// # Panics
///
/// In a build with `debug_assertions`, dropping one panics while the thread
/// holds an [`audio_thread`] mark.
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
/// assert_eq!(shape.get(), Shape::Saw);
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
pub struct Choice<E>(Arc<ParamCell>, PhantomData<fn() -> E>);

impl Float {
    /// Returns the value now, in real units. One atomic load: any thread.
    #[cfg_attr(seam_rtsan, sanitize(realtime = "nonblocking"))]
    pub fn get(&self) -> f32 {
        self.0.value()
    }

    /// Returns its place in the set it was declared in, which the calls
    /// that take a [`ParamId`] name it by.
    pub fn id(&self) -> ParamId {
        self.0.id()
    }
}

impl Bool {
    /// Returns the value now: on at 0.5 and above, as a bool's write snaps.
    /// One atomic load: any thread.
    #[cfg_attr(seam_rtsan, sanitize(realtime = "nonblocking"))]
    pub fn get(&self) -> bool {
        decode_bool(self.0.value())
    }

    /// Returns its place in the set it was declared in.
    pub fn id(&self) -> ParamId {
        self.0.id()
    }
}

impl<E: Choices> Choice<E> {
    /// Returns the value now, as the plugin's enum. One atomic load and
    /// [`Choices::from_index`]: any thread.
    #[cfg_attr(seam_rtsan, sanitize(realtime = "nonblocking"))]
    pub fn get(&self) -> E {
        decode_choice(self.0.value())
    }
}

impl<E> Choice<E> {
    /// Returns its place in the set it was declared in.
    pub fn id(&self) -> ParamId {
        self.0.id()
    }
}

/// On at 0.5 and above, as a bool's write snaps; NaN is off.
fn decode_bool(stored: f32) -> bool {
    stored >= 0.5
}

fn encode_bool(value: bool) -> f32 {
    if value { 1.0 } else { 0.0 }
}

/// The value at the stored index. A choice stores whole indices; any other
/// `f32` truncates, saturating at both ends, and NaN reads as 0.
fn decode_choice<E: Choices>(stored: f32) -> E {
    // `as` from a float saturates and maps NaN to 0, so no index panics.
    E::from_index(stored as usize)
}

fn encode_choice<E: Choices>(value: E) -> f32 {
    value.index() as f32
}

// Written by hand: a derive would make `Choice<E>`'s impls ask the same of
// `E`, which it never holds, and would compare stores by value, not identity.
impl Clone for Float {
    fn clone(&self) -> Self {
        Float(Arc::clone(&self.0))
    }
}

impl Clone for Bool {
    fn clone(&self) -> Self {
        Bool(Arc::clone(&self.0))
    }
}

impl<E> Clone for Choice<E> {
    fn clone(&self) -> Self {
        Choice(Arc::clone(&self.0), PhantomData)
    }
}

impl Drop for Float {
    fn drop(&mut self) {
        audio_thread::assert_not_here("dropping a Float");
    }
}

impl Drop for Bool {
    fn drop(&mut self) {
        audio_thread::assert_not_here("dropping a Bool");
    }
}

impl<E> Drop for Choice<E> {
    fn drop(&mut self) {
        audio_thread::assert_not_here("dropping a Choice");
    }
}

/// The fields a typed parameter shows: which parameter, not its value.
fn debug(f: &mut fmt::Formatter<'_>, name: &str, cell: &ParamCell) -> fmt::Result {
    f.debug_struct(name)
        .field("id", &cell.id())
        .field("key", &cell.decl().key)
        .finish()
}

impl fmt::Debug for Float {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        debug(f, "Float", &self.0)
    }
}

impl fmt::Debug for Bool {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        debug(f, "Bool", &self.0)
    }
}

impl<E> fmt::Debug for Choice<E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        debug(f, "Choice", &self.0)
    }
}

impl PartialEq for Float {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

impl PartialEq for Bool {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

impl<E> PartialEq for Choice<E> {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

impl Eq for Float {}
impl Eq for Bool {}
impl<E> Eq for Choice<E> {}

impl Hash for Float {
    fn hash<H: Hasher>(&self, state: &mut H) {
        Arc::as_ptr(&self.0).hash(state);
    }
}

impl Hash for Bool {
    fn hash<H: Hasher>(&self, state: &mut H) {
        Arc::as_ptr(&self.0).hash(state);
    }
}

impl<E> Hash for Choice<E> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        Arc::as_ptr(&self.0).hash(state);
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
    /// calls it on every read. A choice only ever stores an index below the
    /// labels' count, so the value past it is the plugin's choice; the last
    /// is the usual one.
    fn from_index(index: usize) -> Self;

    /// Returns the value's index into [`LABELS`](Self::LABELS).
    fn index(self) -> usize;
}

mod sealed {
    /// Keeps [`Param`](super::Param) and [`Writable`](super::Writable) to
    /// the types this crate gives them.
    pub trait Sealed {}
}

impl sealed::Sealed for Float {}
impl sealed::Sealed for Bool {}
impl<E> sealed::Sealed for Choice<E> {}
impl sealed::Sealed for &Float {}
impl sealed::Sealed for &Bool {}
impl<E> sealed::Sealed for &Choice<E> {}
impl sealed::Sealed for ParamId {}

/// A typed parameter of any kind: [`Float`], [`Bool`] or [`Choice`], as
/// [`Changes::moved`](crate::Changes::moved) takes a group of them.
///
/// It is sealed: only this crate's parameters implement it.
///
/// # Examples
///
/// ```
/// use seam_params::{Bool, Float, Kind, Param, ParamDecl, ParamSet};
///
/// let set = ParamSet::from_decls([
///     ParamDecl::new("gain", "Gain", Kind::continuous(-60.0, 12.0, 0.1, None), 0.0),
///     ParamDecl::new("on", "On", Kind::Bool, 1.0),
/// ])?;
/// let gain: Float = set.bind("gain")?;
/// let on: Bool = set.bind("on")?;
///
/// // Two kinds in one list.
/// let members: [&dyn Param; 2] = [&gain, &on];
/// assert_eq!(members.len(), 2);
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
pub trait Param: sealed::Sealed {
    /// Returns which store this is, as an address, and how many writes it
    /// has taken.
    #[doc(hidden)]
    fn stamp(&self) -> (usize, u64);
}

/// The stamp of the store `cell` is.
fn stamp(cell: &Arc<ParamCell>) -> (usize, u64) {
    (Arc::as_ptr(cell).addr(), cell.changes())
}

impl Param for Float {
    fn stamp(&self) -> (usize, u64) {
        stamp(&self.0)
    }
}

impl Param for Bool {
    fn stamp(&self) -> (usize, u64) {
        stamp(&self.0)
    }
}

impl<E> Param for Choice<E> {
    fn stamp(&self) -> (usize, u64) {
        stamp(&self.0)
    }
}

/// What [`UiParams::write`] takes: a typed parameter, by reference, with a
/// value of its type, or a [`ParamId`] with the stored `f32`.
///
/// It is sealed: only this crate's parameters and [`ParamId`] implement it.
///
/// # Examples
///
/// ```
/// use seam_params::{Bool, Kind, ParamDecl, ParamSet, UiParams, Writable};
///
/// /// Turns any bool-valued parameter on.
/// fn switch_on<P: Writable<Value = bool>>(ui: &UiParams, param: P) -> bool {
///     ui.write(param, true)
/// }
///
/// let set = ParamSet::from_decls([ParamDecl::new("on", "On", Kind::Bool, 0.0)])?;
/// let on: Bool = set.bind("on")?;
/// let ui = set.take_ui().ok_or("the first UI handle")?;
/// assert!(switch_on(&ui, &on));
/// assert!(on.get());
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
pub trait Writable: sealed::Sealed {
    /// What the write takes: `f32` in real units, `bool` or the plugin's
    /// enum.
    type Value;

    /// Makes the user's write through `ui`.
    #[doc(hidden)]
    fn write_through(self, ui: &UiParams, value: Self::Value) -> bool;
}

impl Writable for ParamId {
    type Value = f32;

    fn write_through(self, ui: &UiParams, value: f32) -> bool {
        ui.user_write(self, value)
    }
}

impl Writable for &Float {
    type Value = f32;

    fn write_through(self, ui: &UiParams, value: f32) -> bool {
        ui.user_write_cell(&self.0, value)
    }
}

impl Writable for &Bool {
    type Value = bool;

    fn write_through(self, ui: &UiParams, value: bool) -> bool {
        ui.user_write_cell(&self.0, encode_bool(value))
    }
}

impl<E: Choices> Writable for &Choice<E> {
    type Value = E;

    fn write_through(self, ui: &UiParams, value: E) -> bool {
        ui.user_write_cell(&self.0, encode_choice(value))
    }
}

/// A typed parameter [`ParamSet::bind`] can make from a key, checking the
/// declaration first.
///
/// [`Float`], [`Bool`] and [`Choice`] implement it. A plugin's own reading
/// of a parameter, say a gain read as a factor, is a newtype that binds one
/// of them and reads it its own way.
///
/// # Examples
///
/// ```
/// use seam_params::{Bind, BindError, Float, Kind, ParamDecl, ParamSet};
///
/// /// A gain in dB, read as the factor a block multiplies by.
/// #[derive(Debug, Clone)]
/// struct Factor(Float);
///
/// impl Factor {
///     fn get(&self) -> f32 {
///         10.0_f32.powf(self.0.get() / 20.0)
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
/// assert!((gain.get() - 0.1).abs() < 1e-6);
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
pub trait Bind: Sized {
    /// Returns the parameter declared under `key` in `set`.
    ///
    /// # Errors
    ///
    /// Returns the [`BindError`] that says why the declaration does not
    /// match the type.
    fn bind(set: &ParamSet, key: &'static str) -> Result<Self, BindError>;
}

/// The cell declared under `key`, and its kind.
fn declared<'s>(
    set: &'s ParamSet,
    key: &'static str,
) -> Result<(&'s Arc<ParamCell>, Kind), BindError> {
    let id = set.id(key).ok_or(BindError::NotDeclared(key))?;
    let cell = set.cell(id);
    Ok((cell, cell.decl().kind))
}

impl Bind for Float {
    fn bind(set: &ParamSet, key: &'static str) -> Result<Self, BindError> {
        match declared(set, key)? {
            (cell, Kind::Continuous(_)) => Ok(Float(Arc::clone(cell))),
            _ => Err(BindError::WrongKind(key)),
        }
    }
}

impl Bind for Bool {
    fn bind(set: &ParamSet, key: &'static str) -> Result<Self, BindError> {
        match declared(set, key)? {
            (cell, Kind::Bool) => Ok(Bool(Arc::clone(cell))),
            _ => Err(BindError::WrongKind(key)),
        }
    }
}

impl<E: Choices> Bind for Choice<E> {
    fn bind(set: &ParamSet, key: &'static str) -> Result<Self, BindError> {
        match declared(set, key)? {
            (cell, Kind::Choice(labels)) if labels == E::LABELS => {
                Ok(Choice(Arc::clone(cell), PhantomData))
            }
            (_, Kind::Choice(_)) => Err(BindError::LabelsDiffer(key)),
            _ => Err(BindError::WrongKind(key)),
        }
    }
}

/// Why a parameter could not be bound: a mistake in the plugin's own
/// declarations or types, found when the plugin is built, never in a block.
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
    /// The parameter is declared as another kind than the type reads.
    WrongKind(&'static str),
    /// The choice's labels are not the enum's [`Choices::LABELS`].
    LabelsDiffer(&'static str),
}

impl fmt::Display for BindError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            BindError::NotDeclared(key) => write!(f, "no parameter is declared as {key:?}"),
            BindError::WrongKind(key) => {
                write!(f, "{key:?} is declared as another kind than the type reads")
            }
            BindError::LabelsDiffer(key) => {
                write!(f, "the labels of choice {key:?} are not the enum's")
            }
        }
    }
}

impl std::error::Error for BindError {}

impl ParamSet {
    /// Binds a typed parameter to the one declared under `key`, checking
    /// that its kind is the type's, and for a [`Choice`] that its labels are
    /// the enum's. Nothing is checked after this: bind once, when the plugin
    /// is built, and keep the parameter.
    ///
    /// # Errors
    ///
    /// Returns the [`BindError`] that names the key.
    ///
    /// # Examples
    ///
    /// A plugin binds a struct of parameters in one function, each field's
    /// type saying which to make.
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
    /// assert!(!params.bypass.get());
    /// assert_eq!(params.gain.get(), 0.0);
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    pub fn bind<P: Bind>(&self, key: &'static str) -> Result<P, BindError> {
        P::bind(self, key)
    }
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;
    use proptest::test_runner::FileFailurePersistence;

    use super::*;

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum Three {
        A,
        B,
        C,
    }

    impl Choices for Three {
        const LABELS: &'static [&'static str] = &["A", "B", "C"];

        fn from_index(index: usize) -> Self {
            match index {
                0 => Three::A,
                1 => Three::B,
                _ => Three::C,
            }
        }

        fn index(self) -> usize {
            self as usize
        }
    }

    #[test]
    fn the_last_parameter_dropped_frees_its_store_after_the_set_has_gone() {
        let set = ParamSet::from_decls([crate::ParamDecl::new("on", "On", Kind::Bool, 0.0)])
            .expect("valid");
        let on: Bool = set.bind("on").expect("a bool");
        let store = Arc::downgrade(&on.0);
        drop(set);
        assert!(store.upgrade().is_some(), "the parameter keeps its store");
        std::thread::scope(|scope| {
            scope.spawn(move || drop(on));
        });
        assert!(store.upgrade().is_none(), "the last drop freed it");
    }

    proptest! {
        #![proptest_config(ProptestConfig {
            failure_persistence: Some(Box::new(FileFailurePersistence::Direct(
                "tests/param.proptest-regressions",
            ))),
            ..ProptestConfig::default()
        })]

        /// Decoding gives a value for every `f32`, each NaN and both
        /// infinities included, and on a stored value it is the inverse of
        /// encoding.
        #[test]
        fn decoding_is_total_and_inverts_encoding_on_stored_values(bits in any::<u32>()) {
            let value = f32::from_bits(bits);

            prop_assert_eq!(decode_bool(value), value >= 0.5);
            let three: Three = decode_choice(value);
            prop_assert!(three.index() < Three::LABELS.len());

            // What a store holds: the snapped value, or the start for NaN.
            let stored = Kind::Bool.snap(value).unwrap_or(0.0);
            let again = encode_bool(decode_bool(stored));
            prop_assert_eq!(again.to_bits(), stored.to_bits());
            let stored = Kind::Choice(Three::LABELS).snap(value).unwrap_or(0.0);
            let again = encode_choice(decode_choice::<Three>(stored));
            prop_assert_eq!(again.to_bits(), stored.to_bits());
        }
    }
}
