//! The mistakes a typed declaration refuses while compiling, as doc tests.
//!
//! Each `compile_fail` example sits beside a twin with the mistake fixed,
//! which compiles: a `compile_fail` example passes on any error at all, so
//! the twin is what shows the error is the one meant.

#![allow(dead_code, reason = "items that exist only to carry doc tests")]

/// A range whose start is above its end.
///
/// ```compile_fail
/// const GAIN: seam_params::FloatDecl =
///     seam_params::FloatDecl::new("gain", "Gain", 12.0..=-60.0, 0.0).checked();
/// ```
///
/// ```
/// const GAIN: seam_params::FloatDecl =
///     seam_params::FloatDecl::new("gain", "Gain", -60.0..=12.0, 0.0).checked();
/// ```
struct RangeBackwards;

/// A default outside its range.
///
/// ```compile_fail
/// const GAIN: seam_params::FloatDecl =
///     seam_params::FloatDecl::new("gain", "Gain", -60.0..=12.0, 24.0).checked();
/// ```
///
/// ```
/// const GAIN: seam_params::FloatDecl =
///     seam_params::FloatDecl::new("gain", "Gain", -60.0..=12.0, 12.0).checked();
/// ```
struct DefaultOutside;

/// An empty key.
///
/// ```compile_fail
/// const ON: seam_params::BoolDecl = seam_params::BoolDecl::new("", "On", true).checked();
/// ```
///
/// ```
/// const ON: seam_params::BoolDecl = seam_params::BoolDecl::new("on", "On", true).checked();
/// ```
struct EmptyKey;

/// A key with a `/`.
///
/// ```compile_fail
/// const ON: seam_params::BoolDecl = seam_params::BoolDecl::new("fx/on", "On", true).checked();
/// ```
///
/// ```
/// const ON: seam_params::BoolDecl = seam_params::BoolDecl::new("fx.on", "On", true).checked();
/// ```
struct KeyWithSlash;

/// A key over 31 bytes.
///
/// ```compile_fail
/// const ON: seam_params::BoolDecl =
///     seam_params::BoolDecl::new("a_key_that_runs_to_thirty_two_by", "On", true).checked();
/// ```
///
/// ```
/// const ON: seam_params::BoolDecl =
///     seam_params::BoolDecl::new("a_key_that_runs_to_thirty_one_b", "On", true).checked();
/// ```
struct KeyTooLong;

/// A choice with one label.
///
/// ```compile_fail
/// # use seam_params::{ChoiceDecl, Choices};
/// #[derive(Debug, Clone, Copy)]
/// struct Only;
///
/// impl Choices for Only {
///     const LABELS: &'static [&'static str] = &["Only"];
///     fn from_index(_: usize) -> Self { Only }
///     fn index(self) -> usize { 0 }
/// }
///
/// const MODE: ChoiceDecl<Only> = ChoiceDecl::new("mode", "Mode", Only).checked();
/// ```
///
/// ```
/// # use seam_params::{ChoiceDecl, Choices};
/// #[derive(Debug, Clone, Copy)]
/// struct Only;
///
/// impl Choices for Only {
///     const LABELS: &'static [&'static str] = &["Only", "Other"];
///     fn from_index(_: usize) -> Self { Only }
///     fn index(self) -> usize { 0 }
/// }
///
/// const MODE: ChoiceDecl<Only> = ChoiceDecl::new("mode", "Mode", Only).checked();
/// ```
struct OneLabel;

/// A choice with two labels that differ only in case.
///
/// ```compile_fail
/// # use seam_params::{ChoiceDecl, Choices};
/// #[derive(Debug, Clone, Copy)]
/// struct Fast;
///
/// impl Choices for Fast {
///     const LABELS: &'static [&'static str] = &["Fast", "FAST"];
///     fn from_index(_: usize) -> Self { Fast }
///     fn index(self) -> usize { 0 }
/// }
///
/// const SPEED: ChoiceDecl<Fast> = ChoiceDecl::new("speed", "Speed", Fast).checked();
/// ```
///
/// ```
/// # use seam_params::{ChoiceDecl, Choices};
/// #[derive(Debug, Clone, Copy)]
/// struct Fast;
///
/// impl Choices for Fast {
///     const LABELS: &'static [&'static str] = &["Fast", "Faster"];
///     fn from_index(_: usize) -> Self { Fast }
///     fn index(self) -> usize { 0 }
/// }
///
/// const SPEED: ChoiceDecl<Fast> = ChoiceDecl::new("speed", "Speed", Fast).checked();
/// ```
struct LabelsDifferInCase;

/// A `Float` field given a `BoolDecl`.
///
/// ```compile_fail
/// use seam_params::{BoolDecl, Declarer, Float, Params};
///
/// struct Gain {
///     gain: Float,
/// }
///
/// impl Params for Gain {
///     fn declare(p: &mut Declarer) -> Self {
///         Gain { gain: p.add(BoolDecl::new("gain", "Gain", false)) }
///     }
/// }
/// ```
///
/// ```
/// use seam_params::{Declarer, Float, FloatDecl, Params};
///
/// struct Gain {
///     gain: Float,
/// }
///
/// impl Params for Gain {
///     fn declare(p: &mut Declarer) -> Self {
///         Gain { gain: p.add(FloatDecl::new("gain", "Gain", -60.0..=12.0, 0.0)) }
///     }
/// }
/// ```
struct FloatGivenBool;

/// A `Choice<A>` field given a `ChoiceDecl<B>`, though the two enums have
/// the same labels.
///
/// ```compile_fail
/// use seam_params::{Choice, ChoiceDecl, Choices, Declarer, Params};
///
/// #[derive(Debug, Clone, Copy)]
/// enum A { One, Two }
/// #[derive(Debug, Clone, Copy)]
/// enum B { One, Two }
///
/// impl Choices for A {
///     const LABELS: &'static [&'static str] = &["One", "Two"];
///     fn from_index(index: usize) -> Self { if index == 0 { A::One } else { A::Two } }
///     fn index(self) -> usize { self as usize }
/// }
///
/// impl Choices for B {
///     const LABELS: &'static [&'static str] = &["One", "Two"];
///     fn from_index(index: usize) -> Self { if index == 0 { B::One } else { B::Two } }
///     fn index(self) -> usize { self as usize }
/// }
///
/// struct Pick {
///     pick: Choice<A>,
/// }
///
/// impl Params for Pick {
///     fn declare(p: &mut Declarer) -> Self {
///         Pick { pick: p.add(ChoiceDecl::new("pick", "Pick", B::One)) }
///     }
/// }
/// ```
///
/// ```
/// use seam_params::{Choice, ChoiceDecl, Choices, Declarer, Params};
///
/// #[derive(Debug, Clone, Copy)]
/// enum A { One, Two }
/// #[derive(Debug, Clone, Copy)]
/// enum B { One, Two }
///
/// impl Choices for A {
///     const LABELS: &'static [&'static str] = &["One", "Two"];
///     fn from_index(index: usize) -> Self { if index == 0 { A::One } else { A::Two } }
///     fn index(self) -> usize { self as usize }
/// }
///
/// impl Choices for B {
///     const LABELS: &'static [&'static str] = &["One", "Two"];
///     fn from_index(index: usize) -> Self { if index == 0 { B::One } else { B::Two } }
///     fn index(self) -> usize { self as usize }
/// }
///
/// struct Pick {
///     pick: Choice<A>,
/// }
///
/// impl Params for Pick {
///     fn declare(p: &mut Declarer) -> Self {
///         Pick { pick: p.add(ChoiceDecl::new("pick", "Pick", A::One)) }
///     }
/// }
/// ```
struct ChoiceGivenOtherEnum;

/// `as_bypass` on a `FloatDecl`: only a bool can be the bypass.
///
/// ```compile_fail
/// const BYPASS: seam_params::FloatDecl =
///     seam_params::FloatDecl::new("bypass", "Bypass", 0.0..=1.0, 0.0).as_bypass();
/// ```
///
/// ```
/// const BYPASS: seam_params::BoolDecl =
///     seam_params::BoolDecl::new("bypass", "Bypass", false).as_bypass();
/// ```
struct BypassOnFloat;
