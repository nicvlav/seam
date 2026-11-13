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
use crate::{Kind, ParamId, ParamSet, audio_thread};

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
pub struct Float(pub(crate) Arc<ParamCell>);

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
pub struct Bool(pub(crate) Arc<ParamCell>);

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
pub struct Choice<E>(pub(crate) Arc<ParamCell>, pub(crate) PhantomData<fn() -> E>);

// Each typed parameter is made through `from_cell`, so the cell counts it:
// on declaring, on binding and on cloning.
impl Float {
    pub(crate) fn from_cell(cell: Arc<ParamCell>) -> Self {
        cell.hold();
        Float(cell)
    }
}

impl Bool {
    pub(crate) fn from_cell(cell: Arc<ParamCell>) -> Self {
        cell.hold();
        Bool(cell)
    }
}

impl<E> Choice<E> {
    pub(crate) fn from_cell(cell: Arc<ParamCell>) -> Self {
        cell.hold();
        Choice(cell, PhantomData)
    }
}

impl Float {
    /// Returns the value now, in real units. One atomic load: any thread.
    #[cfg_attr(seam_rtsan, sanitize(realtime = "nonblocking"))]
    pub fn get(&self) -> f32 {
        self.0.value()
    }

    /// Returns its place in the set it was declared in, which the calls
    /// that take a [`ParamId`] name it by. Those calls take the parameter
    /// itself too.
    ///
    /// # Panics
    ///
    /// In a build with `debug_assertions`, before its set is built: inside
    /// [`Params::declare`](crate::Params::declare), where no parameter has
    /// its place yet, and for one a refused declaration handed back. And for
    /// a parameter made at run time, which no set lists: pass the parameter
    /// itself instead.
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
    ///
    /// # Panics
    ///
    /// As [`Float::id`] does: before its set is built, and for a parameter
    /// made at run time.
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
    ///
    /// # Panics
    ///
    /// As [`Float::id`] does: before its set is built, and for a parameter
    /// made at run time.
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
/// `f32` truncates, saturating at 0 and at the last label, and NaN reads as 0.
fn decode_choice<E: Choices>(stored: f32) -> E {
    // `as` from a float saturates and maps NaN to 0. The clamp keeps the
    // index below the labels' count, so the audio thread never asks
    // `from_index` for one past them, even if a store held one.
    let last = E::LABELS.len().saturating_sub(1);
    E::from_index((stored as usize).min(last))
}

fn encode_choice<E: Choices>(value: E) -> f32 {
    value.index() as f32
}

// Written by hand: a derive would make `Choice<E>`'s impls ask the same of
// `E`, which it never holds, and would compare stores by value, not identity.
impl Clone for Float {
    fn clone(&self) -> Self {
        Float::from_cell(Arc::clone(&self.0))
    }
}

impl Clone for Bool {
    fn clone(&self) -> Self {
        Bool::from_cell(Arc::clone(&self.0))
    }
}

impl<E> Clone for Choice<E> {
    fn clone(&self) -> Self {
        Choice::from_cell(Arc::clone(&self.0))
    }
}

impl Drop for Float {
    fn drop(&mut self) {
        self.0.let_go();
        audio_thread::assert_not_here("dropping a Float");
    }
}

impl Drop for Bool {
    fn drop(&mut self) {
        self.0.let_go();
        audio_thread::assert_not_here("dropping a Bool");
    }
}

impl<E> Drop for Choice<E> {
    fn drop(&mut self) {
        self.0.let_go();
        audio_thread::assert_not_here("dropping a Choice");
    }
}

/// The fields a typed parameter shows: which parameter, not its value. The
/// id is left out until its set is built.
fn debug(f: &mut fmt::Formatter<'_>, name: &str, cell: &ParamCell) -> fmt::Result {
    let mut fields = f.debug_struct(name);
    if let Some(id) = cell.placed() {
        fields.field("id", &id);
    }
    fields.field("key", &cell.decl().key).finish()
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

pub(crate) mod sealed {
    /// Keeps [`AnyParam`](super::AnyParam) and
    /// [`Writable`](super::Writable) to the types this crate gives them.
    pub trait Sealed {}

    /// What the plumbing methods of those traits take, so only this crate
    /// can call them: nothing outside it can make one.
    #[derive(Debug, Clone, Copy)]
    pub struct Token(pub(crate) ());
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
/// A group of two kinds, named once and checked each block.
///
/// ```
/// use seam_params::{AnyParam, Bool, Changes, Float, Kind, ParamDecl, ParamSet};
///
/// let set = ParamSet::from_decls([
///     ParamDecl::new("gain", "Gain", Kind::continuous(-60.0, 12.0, 0.1, None), 0.0),
///     ParamDecl::new("on", "On", Kind::Bool, 1.0),
/// ])?;
/// let gain: Float = set.bind("gain")?;
/// let on: Bool = set.bind("on")?;
///
/// let band: [&dyn AnyParam; 2] = [&gain, &on];
/// let mut changes = Changes::new();
/// assert!(changes.moved(band));
/// assert!(!changes.moved(band));
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
pub trait AnyParam: sealed::Sealed {
    /// Returns which store this is, as its cell's serial, and how many
    /// writes it has taken.
    #[doc(hidden)]
    fn stamp(&self, token: sealed::Token) -> (u64, u64);

    /// Returns the parameter with its type erased, as Seam's other crates
    /// hold it.
    #[doc(hidden)]
    fn erased(&self) -> ParamRef;
}

/// A parameter of any kind with its type erased, as Seam's other crates hold
/// one: a learn route, an undo record, the save's list. It does not count as
/// the plugin holding the parameter.
///
/// # Panics
///
/// In a build with `debug_assertions`, dropping one panics while the thread
/// holds an [`audio_thread`] mark.
#[doc(hidden)]
#[derive(Debug)]
pub struct ParamRef(pub(crate) Arc<ParamCell>);

/// A [`ParamRef`] that keeps nothing alive: what an undo step holds.
#[doc(hidden)]
#[derive(Debug, Clone)]
pub struct WeakParamRef(std::sync::Weak<ParamCell>);

impl Clone for ParamRef {
    fn clone(&self) -> Self {
        ParamRef(Arc::clone(&self.0))
    }
}

impl Drop for ParamRef {
    fn drop(&mut self) {
        audio_thread::assert_not_here("dropping a ParamRef");
    }
}

impl ParamRef {
    /// Returns which store this is, for as long as the process runs.
    pub fn serial(&self) -> u64 {
        self.0.serial()
    }

    /// Returns its place in its set, or `None` for one made at run time.
    pub fn listed(&self) -> Option<ParamId> {
        self.0.placed()
    }

    /// Returns its declaration.
    pub fn decl(&self) -> &crate::ParamDecl {
        self.0.decl()
    }

    /// Returns the value now, in real units.
    pub fn value(&self) -> f32 {
        self.0.value()
    }

    /// Returns its change count, then its value and that value's origin, as
    /// [`ParamSet::read`] does for a listed one.
    pub fn read(&self) -> crate::Reading<f32> {
        self.0.read()
    }

    /// Returns whether the plugin still holds it through a typed parameter.
    pub fn held(&self) -> bool {
        self.0.held()
    }

    /// Returns whether `set`'s instance made it at run time.
    pub fn made_by(&self, set: &ParamSet) -> bool {
        set.made(&self.0)
    }

    /// Writes a learned controller's 0 to 1, as
    /// [`AudioParams::learned_write`](crate::AudioParams::learned_write)
    /// does. Never allocates, locks or frees: audio thread.
    #[cfg_attr(seam_rtsan, sanitize(realtime = "nonblocking"))]
    pub fn learned_write(&self, normalized: f64) {
        self.0.learned_write(normalized);
    }

    /// Returns a link that keeps it alive no longer.
    pub fn downgrade(&self) -> WeakParamRef {
        WeakParamRef(Arc::downgrade(&self.0))
    }
}

impl WeakParamRef {
    /// Returns the parameter while anything still keeps it alive.
    pub fn upgrade(&self) -> Option<ParamRef> {
        self.0.upgrade().map(ParamRef)
    }
}

/// The stamp of the store `cell` is.
fn stamp(cell: &ParamCell) -> (u64, u64) {
    (cell.serial(), cell.changes())
}

impl AnyParam for Float {
    fn stamp(&self, _: sealed::Token) -> (u64, u64) {
        stamp(&self.0)
    }

    fn erased(&self) -> ParamRef {
        ParamRef(Arc::clone(&self.0))
    }
}

impl AnyParam for Bool {
    fn stamp(&self, _: sealed::Token) -> (u64, u64) {
        stamp(&self.0)
    }

    fn erased(&self) -> ParamRef {
        ParamRef(Arc::clone(&self.0))
    }
}

impl<E> AnyParam for Choice<E> {
    fn stamp(&self, _: sealed::Token) -> (u64, u64) {
        stamp(&self.0)
    }

    fn erased(&self) -> ParamRef {
        ParamRef(Arc::clone(&self.0))
    }
}

/// What [`UiParams::write`](crate::UiParams::write) and the other calls that name one parameter
/// take: a typed parameter, by reference, with a value of its type, or a
/// [`ParamId`] with the stored `f32`.
///
/// A typed parameter reaches one the host lists and one made at run time
/// alike; a [`ParamId`] reaches only a listed one. It is sealed: only this
/// crate's parameters and [`ParamId`] implement it.
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

    /// Names the parameter, for [`UiParams`](crate::UiParams) to find.
    #[doc(hidden)]
    fn named(&self, token: sealed::Token) -> Named<'_>;

    /// The `f32` a store holds for `value`.
    #[doc(hidden)]
    fn encode(value: Self::Value, token: sealed::Token) -> f32;

    /// The value a store's `f32` reads as.
    #[doc(hidden)]
    fn decode(stored: f32, token: sealed::Token) -> Self::Value;
}

/// A parameter as a [`Writable`] names it, for [`UiParams`](crate::UiParams) to find among
/// its own.
#[doc(hidden)]
#[derive(Debug, Clone, Copy)]
pub struct Named<'a>(pub(crate) Naming<'a>);

/// How a [`Writable`] names its parameter: by place, or by store.
#[derive(Debug, Clone, Copy)]
pub(crate) enum Naming<'a> {
    Id(ParamId),
    Store(&'a Arc<ParamCell>),
}

impl Writable for ParamId {
    type Value = f32;

    fn named(&self, _: sealed::Token) -> Named<'_> {
        Named(Naming::Id(*self))
    }

    fn encode(value: f32, _: sealed::Token) -> f32 {
        value
    }

    fn decode(stored: f32, _: sealed::Token) -> f32 {
        stored
    }
}

impl Writable for &Float {
    type Value = f32;

    fn named(&self, _: sealed::Token) -> Named<'_> {
        Named(Naming::Store(&self.0))
    }

    fn encode(value: f32, _: sealed::Token) -> f32 {
        value
    }

    fn decode(stored: f32, _: sealed::Token) -> f32 {
        stored
    }
}

impl Writable for &Bool {
    type Value = bool;

    fn named(&self, _: sealed::Token) -> Named<'_> {
        Named(Naming::Store(&self.0))
    }

    fn encode(value: bool, _: sealed::Token) -> f32 {
        encode_bool(value)
    }

    fn decode(stored: f32, _: sealed::Token) -> bool {
        decode_bool(stored)
    }
}

impl<E: Choices> Writable for &Choice<E> {
    type Value = E;

    fn named(&self, _: sealed::Token) -> Named<'_> {
        Named(Naming::Store(&self.0))
    }

    fn encode(value: E, _: sealed::Token) -> f32 {
        encode_choice(value)
    }

    fn decode(stored: f32, _: sealed::Token) -> E {
        decode_choice(stored)
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
            (cell, Kind::Continuous(_)) => Ok(Float::from_cell(Arc::clone(cell))),
            _ => Err(BindError::WrongKind(key)),
        }
    }
}

impl Bind for Bool {
    fn bind(set: &ParamSet, key: &'static str) -> Result<Self, BindError> {
        match declared(set, key)? {
            (cell, Kind::Bool) => Ok(Bool::from_cell(Arc::clone(cell))),
            _ => Err(BindError::WrongKind(key)),
        }
    }
}

impl<E: Choices> Bind for Choice<E> {
    fn bind(set: &ParamSet, key: &'static str) -> Result<Self, BindError> {
        match declared(set, key)? {
            (cell, Kind::Choice(labels)) if labels == E::LABELS => {
                Ok(Choice::from_cell(Arc::clone(cell)))
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
