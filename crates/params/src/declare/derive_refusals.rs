//! The mistakes `#[derive(Params)]` and `#[derive(Choices)]` refuse while
//! compiling, as doc tests.
//!
//! Each `compile_fail` example sits beside a twin with the mistake fixed,
//! which compiles: a `compile_fail` example passes on any error at all, so
//! the twin, and the error code where the compiler gives one, show the error
//! is the one meant. The derive's own errors have no code.

#![allow(dead_code, reason = "items that exist only to carry doc tests")]

/// A float's arguments on a `Bool` field.
///
/// ```compile_fail,E0061
/// use seam_params::{Bool, Params};
///
/// #[derive(Params)]
/// struct P {
///     #[param("on", "On", 0.0..=1.0, 0.0)]
///     on: Bool,
/// }
/// ```
///
/// ```
/// use seam_params::{Bool, Params};
///
/// #[derive(Params)]
/// struct P {
///     #[param("on", "On", false)]
///     on: Bool,
/// }
/// ```
struct FloatArgumentsOnBool;

/// An option the kind does not have: a bool has no step.
///
/// ```compile_fail,E0599
/// use seam_params::{Bool, Params};
///
/// #[derive(Params)]
/// struct P {
///     #[param("on", "On", false, step = 1.0)]
///     on: Bool,
/// }
/// ```
///
/// ```
/// use seam_params::{Bool, Params};
///
/// #[derive(Params)]
/// struct P {
///     #[param("on", "On", false, group = "Mix")]
///     on: Bool,
/// }
/// ```
struct OptionTheKindLacks;

/// A flag the kind does not have: only a bool can be the bypass.
///
/// ```compile_fail,E0599
/// use seam_params::{Float, Params};
///
/// #[derive(Params)]
/// struct P {
///     #[param("bypass", "Bypass", 0.0..=1.0, 0.0, bypass)]
///     bypass: Float,
/// }
/// ```
///
/// ```
/// use seam_params::{Bool, Params};
///
/// #[derive(Params)]
/// struct P {
///     #[param("bypass", "Bypass", false, bypass)]
///     bypass: Bool,
/// }
/// ```
struct FlagTheKindLacks;

/// An option no declaration has, which the derive names with the ones there
/// are.
///
/// ```compile_fail
/// use seam_params::{Float, Params};
///
/// #[derive(Params)]
/// struct P {
///     #[param("gain", "Gain", -60.0..=12.0, 0.0, stepp = 0.1)]
///     gain: Float,
/// }
/// ```
///
/// ```
/// use seam_params::{Float, Params};
///
/// #[derive(Params)]
/// struct P {
///     #[param("gain", "Gain", -60.0..=12.0, 0.0, step = 0.1)]
///     gain: Float,
/// }
/// ```
struct MisspeltOption;

/// Another enum's value for a `Choice<E>`.
///
/// ```compile_fail,E0308
/// # use seam_params::Choices;
/// # #[derive(Debug, Clone, Copy)]
/// # enum Mode { Clean, Warm }
/// # #[derive(Debug, Clone, Copy)]
/// # enum Other { Clean, Warm }
/// # impl Choices for Mode {
/// #     const LABELS: &'static [&'static str] = &["Clean", "Warm"];
/// #     fn from_index(index: usize) -> Self { if index == 0 { Mode::Clean } else { Mode::Warm } }
/// #     fn index(self) -> usize { self as usize }
/// # }
/// use seam_params::{Choice, Params};
///
/// #[derive(Params)]
/// struct P {
///     #[param("mode", "Mode", Other::Clean)]
///     mode: Choice<Mode>,
/// }
/// ```
///
/// ```
/// # use seam_params::Choices;
/// # #[derive(Debug, Clone, Copy)]
/// # enum Mode { Clean, Warm }
/// # #[derive(Debug, Clone, Copy)]
/// # enum Other { Clean, Warm }
/// # impl Choices for Mode {
/// #     const LABELS: &'static [&'static str] = &["Clean", "Warm"];
/// #     fn from_index(index: usize) -> Self { if index == 0 { Mode::Clean } else { Mode::Warm } }
/// #     fn index(self) -> usize { self as usize }
/// # }
/// use seam_params::{Choice, Params};
///
/// #[derive(Params)]
/// struct P {
///     #[param("mode", "Mode", Mode::Clean)]
///     mode: Choice<Mode>,
/// }
/// ```
struct ChoiceGivenOtherEnum;

/// An argument after an option.
///
/// ```compile_fail
/// use seam_params::{Float, Params};
///
/// #[derive(Params)]
/// struct P {
///     #[param("gain", "Gain", -60.0..=12.0, unit = "dB", 0.0)]
///     gain: Float,
/// }
/// ```
///
/// ```
/// use seam_params::{Float, Params};
///
/// #[derive(Params)]
/// struct P {
///     #[param("gain", "Gain", -60.0..=12.0, 0.0, unit = "dB")]
///     gain: Float,
/// }
/// ```
struct ArgumentAfterOption;

/// A range whose start is above its end.
///
/// ```compile_fail,E0080
/// use seam_params::{Float, Params};
///
/// #[derive(Params)]
/// struct P {
///     #[param("gain", "Gain", 12.0..=-60.0, 0.0)]
///     gain: Float,
/// }
/// ```
///
/// ```
/// use seam_params::{Float, Params};
///
/// #[derive(Params)]
/// struct P {
///     #[param("gain", "Gain", -60.0..=12.0, 0.0)]
///     gain: Float,
/// }
/// ```
struct RangeBackwards;

/// A default outside its range.
///
/// ```compile_fail,E0080
/// use seam_params::{Float, Params};
///
/// #[derive(Params)]
/// struct P {
///     #[param("gain", "Gain", -60.0..=12.0, 24.0)]
///     gain: Float,
/// }
/// ```
///
/// ```
/// use seam_params::{Float, Params};
///
/// #[derive(Params)]
/// struct P {
///     #[param("gain", "Gain", -60.0..=12.0, 12.0)]
///     gain: Float,
/// }
/// ```
struct DefaultOutside;

/// A key with a `/`.
///
/// ```compile_fail,E0080
/// use seam_params::{Bool, Params};
///
/// #[derive(Params)]
/// struct P {
///     #[param("fx/on", "On", true)]
///     on: Bool,
/// }
/// ```
///
/// ```
/// use seam_params::{Bool, Params};
///
/// #[derive(Params)]
/// struct P {
///     #[param("fx.on", "On", true)]
///     on: Bool,
/// }
/// ```
struct KeyWithSlash;

/// Two fields with one key.
///
/// ```compile_fail,E0080
/// use seam_params::{Bool, Float, Params};
///
/// #[derive(Params)]
/// struct P {
///     #[param("gain", "Gain", -60.0..=12.0, 0.0)]
///     gain: Float,
///     #[param("on", "On", true)]
///     on: Bool,
///     #[param("gain", "Gain on", true)]
///     gain_on: Bool,
/// }
/// ```
///
/// ```
/// use seam_params::{Bool, Float, Params};
///
/// #[derive(Params)]
/// struct P {
///     #[param("gain", "Gain", -60.0..=12.0, 0.0)]
///     gain: Float,
///     #[param("on", "On", true)]
///     on: Bool,
///     #[param("gain.on", "Gain on", true)]
///     gain_on: Bool,
/// }
/// ```
struct KeyTwice;

/// A field that is neither a parameter nor a struct of them.
///
/// ```compile_fail,E0277
/// use seam_params::{Float, Params};
///
/// #[derive(Params)]
/// struct P {
///     #[param("gain", "Gain", -60.0..=12.0, 0.0)]
///     gain: Float,
///     label: String,
/// }
/// ```
///
/// ```
/// use seam_params::{Float, Params};
///
/// #[derive(Params)]
/// struct Inner {
///     #[param("trim", "Trim", -12.0..=12.0, 0.0)]
///     trim: Float,
/// }
///
/// #[derive(Params)]
/// struct P {
///     #[param("gain", "Gain", -60.0..=12.0, 0.0)]
///     gain: Float,
///     inner: Inner,
/// }
/// ```
struct NeitherParameterNorStruct;

/// A mark on a field that is not a parameter.
///
/// ```compile_fail,E0277
/// use seam_params::Params;
///
/// #[derive(Params)]
/// struct P {
///     #[param("label", "Label", false)]
///     label: String,
/// }
/// ```
///
/// ```
/// use seam_params::{Bool, Params};
///
/// #[derive(Params)]
/// struct P {
///     #[param("label", "Label", false)]
///     label: Bool,
/// }
/// ```
struct MarkOnOtherType;

/// The derive on an enum.
///
/// ```compile_fail
/// use seam_params::{Float, Params};
///
/// #[derive(Params)]
/// enum P {
///     Gain(Float),
/// }
/// ```
///
/// ```
/// use seam_params::{Float, Params};
///
/// #[derive(Params)]
/// struct P {
///     #[param("gain", "Gain", -60.0..=12.0, 0.0)]
///     gain: Float,
/// }
/// ```
struct OnEnum;

/// The derive on a tuple struct.
///
/// ```compile_fail
/// use seam_params::{Float, Params};
///
/// #[derive(Params)]
/// struct P(#[param("gain", "Gain", -60.0..=12.0, 0.0)] Float);
/// ```
///
/// ```
/// use seam_params::{Float, Params};
///
/// #[derive(Params)]
/// struct P {
///     #[param("gain", "Gain", -60.0..=12.0, 0.0)]
///     gain: Float,
/// }
/// ```
struct OnTupleStruct;

/// The derive on a generic struct.
///
/// ```compile_fail
/// use seam_params::{Float, Params};
///
/// #[derive(Params)]
/// struct P<T: Params> {
///     #[param("gain", "Gain", -60.0..=12.0, 0.0)]
///     gain: Float,
///     inner: T,
/// }
/// ```
///
/// ```
/// use seam_params::{Float, Params};
///
/// #[derive(Params)]
/// struct P {
///     #[param("gain", "Gain", -60.0..=12.0, 0.0)]
///     gain: Float,
///     inner: (),
/// }
/// ```
struct OnGenericStruct;

/// `Choices` derived for a struct.
///
/// ```compile_fail
/// use seam_params::Choices;
///
/// #[derive(Clone, Copy, Choices)]
/// struct Mode {
///     index: usize,
/// }
/// ```
///
/// ```
/// use seam_params::Choices;
///
/// #[derive(Clone, Copy, Choices)]
/// enum Mode {
///     Clean,
///     Warm,
/// }
/// ```
struct ChoicesOnStruct;

/// `Choices` derived for a union.
///
/// ```compile_fail
/// use seam_params::Choices;
///
/// #[derive(Clone, Copy, Choices)]
/// union Mode {
///     index: usize,
/// }
/// ```
///
/// ```
/// use seam_params::Choices;
///
/// #[derive(Clone, Copy, Choices)]
/// enum Mode {
///     Clean,
///     Warm,
/// }
/// ```
struct ChoicesOnUnion;

/// A variant with fields.
///
/// ```compile_fail
/// use seam_params::Choices;
///
/// #[derive(Clone, Copy, Choices)]
/// enum Mode {
///     Clean,
///     Warm(u8),
/// }
/// ```
///
/// ```
/// use seam_params::Choices;
///
/// #[derive(Clone, Copy, Choices)]
/// enum Mode {
///     Clean,
///     Warm,
/// }
/// ```
struct VariantWithFields;

/// A generic enum.
///
/// ```compile_fail
/// use std::marker::PhantomData;
///
/// use seam_params::Choices;
///
/// #[derive(Clone, Copy, Choices)]
/// enum Mode<T: Copy> {
///     Clean,
///     Warm(PhantomData<T>),
/// }
/// ```
///
/// ```
/// use seam_params::Choices;
///
/// #[derive(Clone, Copy, Choices)]
/// enum Mode {
///     Clean,
///     Warm,
/// }
/// ```
struct GenericEnum;

/// An enum of one variant: a choice has two values at least.
///
/// ```compile_fail
/// use seam_params::Choices;
///
/// #[derive(Clone, Copy, Choices)]
/// enum Mode {
///     Clean,
/// }
/// ```
///
/// ```
/// use seam_params::Choices;
///
/// #[derive(Clone, Copy, Choices)]
/// enum Mode {
///     Clean,
///     Warm,
/// }
/// ```
struct OneVariant;

/// A written discriminant that is not the variant's position, refused on
/// the discriminant. One that is the position compiles.
///
/// ```compile_fail,E0080
/// use seam_params::Choices;
///
/// #[derive(Clone, Copy, Choices)]
/// enum Mode {
///     Clean = 0,
///     Hot = 2,
/// }
/// ```
///
/// ```
/// use seam_params::Choices;
///
/// #[derive(Clone, Copy, Choices)]
/// enum Mode {
///     Clean = 0,
///     Hot = 1,
/// }
/// ```
struct DiscriminantNotPosition;

/// A label with whitespace at an end, refused on the label.
///
/// ```compile_fail,E0080
/// use seam_params::Choices;
///
/// #[derive(Clone, Copy, Choices)]
/// enum Mode {
///     Clean,
///     #[choice("Warm ")]
///     Warm,
/// }
/// ```
///
/// ```
/// use seam_params::Choices;
///
/// #[derive(Clone, Copy, Choices)]
/// enum Mode {
///     Clean,
///     #[choice("Warm too")]
///     Warm,
/// }
/// ```
struct PaddedLabel;

/// An empty label, refused on the label.
///
/// ```compile_fail,E0080
/// use seam_params::Choices;
///
/// #[derive(Clone, Copy, Choices)]
/// enum Mode {
///     Clean,
///     #[choice("")]
///     Warm,
/// }
/// ```
///
/// ```
/// use seam_params::Choices;
///
/// #[derive(Clone, Copy, Choices)]
/// enum Mode {
///     Clean,
///     Warm,
/// }
/// ```
struct EmptyLabel;

/// Two labels that differ only in ASCII case, refused on the second.
///
/// ```compile_fail,E0080
/// use seam_params::Choices;
///
/// #[derive(Clone, Copy, Choices)]
/// enum Mode {
///     #[choice("warm")]
///     Clean,
///     Warm,
/// }
/// ```
///
/// ```
/// use seam_params::Choices;
///
/// #[derive(Clone, Copy, Choices)]
/// enum Mode {
///     #[choice("clean")]
///     Clean,
///     Warm,
/// }
/// ```
struct LabelsDifferInCase;

/// A mark that is not one string.
///
/// ```compile_fail
/// use seam_params::Choices;
///
/// #[derive(Clone, Copy, Choices)]
/// enum Mode {
///     #[choice(12)]
///     Clean,
///     Warm,
/// }
/// ```
///
/// ```
/// use seam_params::Choices;
///
/// #[derive(Clone, Copy, Choices)]
/// enum Mode {
///     #[choice("12")]
///     Clean,
///     Warm,
/// }
/// ```
struct MarkNotAString;

/// Two marks on one variant.
///
/// ```compile_fail
/// use seam_params::Choices;
///
/// #[derive(Clone, Copy, Choices)]
/// enum Mode {
///     #[choice("A")]
///     #[choice("B")]
///     Clean,
///     Warm,
/// }
/// ```
///
/// ```
/// use seam_params::Choices;
///
/// #[derive(Clone, Copy, Choices)]
/// enum Mode {
///     #[choice("A")]
///     Clean,
///     Warm,
/// }
/// ```
struct TwoMarks;

/// A mark on the enum, not on a variant.
///
/// ```compile_fail
/// use seam_params::Choices;
///
/// #[derive(Clone, Copy, Choices)]
/// #[choice("Mode")]
/// enum Mode {
///     Clean,
///     Warm,
/// }
/// ```
///
/// ```
/// use seam_params::Choices;
///
/// #[derive(Clone, Copy, Choices)]
/// enum Mode {
///     Clean,
///     Warm,
/// }
/// ```
struct MarkOnEnum;
