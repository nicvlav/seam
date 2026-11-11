//! Declaring parameters where they are held: a typed declaration says what a
//! parameter is, and declaring it hands the parameter back.

use std::marker::PhantomData;
use std::ops::RangeInclusive;
use std::sync::Arc;

use crate::param::sealed::{Sealed, Token};
use crate::set::ParamCell;
use crate::{
    Bool, Choice, Choices, DeclError, Float, Kind, ParamDecl, ParamSet, ParamSetBuilder, Range,
};

#[cfg(all(doctest, feature = "derive"))]
mod derive_refusals;
#[cfg(doctest)]
mod refusals;
pub(crate) mod rules;

/// A continuous parameter, declared: [`Declarer::add`] makes a [`Float`]
/// from it.
///
/// Its range is written as one, ends included, and its default must lie in
/// it: a default outside is refused, not snapped as a [`ParamDecl`]'s is.
/// [`checked`](Self::checked) refuses a mistake while compiling.
///
/// # Examples
///
/// ```
/// use seam_params::{Declarer, Float, FloatDecl, ParamSet, Params};
///
/// struct Gain {
///     gain: Float,
/// }
///
/// impl Params for Gain {
///     fn declare(p: &mut Declarer) -> Self {
///         Gain {
///             gain: p.add(
///                 FloatDecl::new("gain", "Gain", -60.0..=12.0, 0.0)
///                     .with_step(0.1)
///                     .with_unit("dB"),
///             ),
///         }
///     }
/// }
///
/// let (set, params) = ParamSet::declare::<Gain>()?;
/// assert_eq!(params.gain.get(), 0.0);
/// assert_eq!(set.decl(params.gain.id()).unit, "dB");
/// # Ok::<(), seam_params::DeclError>(())
/// ```
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FloatDecl {
    key: &'static str,
    name: &'static str,
    start: f32,
    end: f32,
    default: f32,
    step: f32,
    skew_centre: Option<f32>,
    unit: &'static str,
    group: &'static str,
}

impl FloatDecl {
    /// Declares a number from `range`'s start to its end that starts at
    /// `default`, under `key`, shown as `name`.
    ///
    /// It is continuous, linear, has no unit and sits at the top level; the
    /// `with_` methods set those.
    pub const fn new(
        key: &'static str,
        name: &'static str,
        range: RangeInclusive<f32>,
        default: f32,
    ) -> Self {
        FloatDecl {
            key,
            name,
            start: *range.start(),
            end: *range.end(),
            default,
            step: 0.0,
            skew_centre: None,
            unit: "",
            group: "",
        }
    }

    /// Snaps it to every `step` from the start; 0 is continuous.
    #[must_use]
    pub const fn with_step(mut self, step: f32) -> Self {
        self.step = step;
        self
    }

    /// Puts `centre` at the middle of a control's travel, a host's 0.5.
    #[must_use]
    pub const fn with_skew_centre(mut self, centre: f32) -> Self {
        self.skew_centre = Some(centre);
        self
    }

    /// Sets the unit shown after the value ("dB", "%").
    #[must_use]
    pub const fn with_unit(mut self, unit: &'static str) -> Self {
        self.unit = unit;
        self
    }

    /// Puts it in a group, a `/`-separated path ("Mix/Sides"), as
    /// [`ParamDecl::with_group`] does.
    #[must_use]
    pub const fn with_group(mut self, group: &'static str) -> Self {
        self.group = group;
        self
    }

    /// Hands the declaration back if a parameter could be made from it, and
    /// panics otherwise. In a `const` that is a build error, on the line the
    /// constant is declared.
    ///
    /// It applies the rules [`Declarer::add`] applies to a declaration on
    /// its own: the key's, the group's, the range's and the default's. What
    /// only the whole set shows, such as two parameters with one key, waits
    /// for the set to be built.
    ///
    /// A named `const` is evaluated by a plain `cargo check`. An inline
    /// `const { }` block inside a function is evaluated only when code is
    /// generated: by `cargo build`, or a test run.
    ///
    /// # Panics
    ///
    /// On the first rule the declaration breaks, with that rule in words.
    ///
    /// # Examples
    ///
    /// ```
    /// use seam_params::FloatDecl;
    ///
    /// const MIX: FloatDecl = FloatDecl::new("mix", "Mix", 0.0..=100.0, 100.0)
    ///     .with_unit("%")
    ///     .checked();
    /// ```
    ///
    /// A default past the end does not build: "a parameter's default is
    /// outside its range".
    ///
    /// ```compile_fail
    /// use seam_params::FloatDecl;
    ///
    /// const MIX: FloatDecl = FloatDecl::new("mix", "Mix", 0.0..=100.0, 150.0)
    ///     .with_unit("%")
    ///     .checked();
    /// ```
    #[must_use]
    pub const fn checked(self) -> Self {
        match self.check() {
            Ok(_) => self,
            Err(error) => panic!("{}", error.rule()),
        }
    }

    /// Its range, or the first rule it breaks on its own.
    const fn check(&self) -> Result<Range, DeclError> {
        match rules::and(rules::key(self.key), rules::group(self.key, self.group)) {
            Ok(()) => rules::range(
                self.key,
                self.start,
                self.end,
                self.step,
                self.skew_centre,
                self.default,
            ),
            Err(error) => Err(error),
        }
    }
}

/// A bool parameter, declared: [`Declarer::add`] makes a [`Bool`] from it.
/// Only a bool can be a plugin's bypass.
///
/// # Examples
///
/// ```
/// use seam_params::{Bool, BoolDecl, Declarer, ParamSet, Params};
///
/// struct Switches {
///     bypass: Bool,
/// }
///
/// impl Params for Switches {
///     fn declare(p: &mut Declarer) -> Self {
///         Switches {
///             bypass: p.add(BoolDecl::new("bypass", "Bypass", false).as_bypass()),
///         }
///     }
/// }
///
/// let (set, params) = ParamSet::declare::<Switches>()?;
/// assert_eq!(set.bypass(), Some(params.bypass.id()));
/// assert!(!params.bypass.get());
/// # Ok::<(), seam_params::DeclError>(())
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BoolDecl {
    key: &'static str,
    name: &'static str,
    default: bool,
    group: &'static str,
    bypass: bool,
}

impl BoolDecl {
    /// Declares a bool that starts at `default`, under `key`, shown as
    /// `name`, at the top level and not the bypass.
    pub const fn new(key: &'static str, name: &'static str, default: bool) -> Self {
        BoolDecl {
            key,
            name,
            default,
            group: "",
            bypass: false,
        }
    }

    /// Puts it in a group, as [`FloatDecl::with_group`] does.
    #[must_use]
    pub const fn with_group(mut self, group: &'static str) -> Self {
        self.group = group;
        self
    }

    /// Makes it the plugin's bypass, as [`ParamDecl::as_bypass`] does: at
    /// most one parameter is.
    #[must_use]
    pub const fn as_bypass(mut self) -> Self {
        self.bypass = true;
        self
    }

    /// Hands the declaration back, or panics on its key or group, as
    /// [`FloatDecl::checked`] does.
    ///
    /// # Panics
    ///
    /// On the first rule the declaration breaks, with that rule in words.
    #[must_use]
    pub const fn checked(self) -> Self {
        match self.check() {
            Ok(()) => self,
            Err(error) => panic!("{}", error.rule()),
        }
    }

    /// The first rule it breaks on its own.
    const fn check(&self) -> Result<(), DeclError> {
        rules::and(rules::key(self.key), rules::group(self.key, self.group))
    }
}

/// A choice parameter, declared: [`Declarer::add`] makes a [`Choice<E>`]
/// from it, whose labels are [`E::LABELS`](Choices::LABELS).
///
/// Its default is a value of `E`. [`checked`](Self::checked) refuses the
/// labels while compiling, but cannot ask the default for its index, since
/// a `const fn` cannot call a trait method: [`Declarer::add`] refuses an
/// index past the labels.
///
/// # Examples
///
/// ```
/// use seam_params::{Choice, ChoiceDecl, Choices, Declarer, ParamSet, Params};
///
/// #[derive(Debug, Clone, Copy, PartialEq)]
/// enum Mode {
///     Clean,
///     Warm,
/// }
///
/// impl Choices for Mode {
///     const LABELS: &'static [&'static str] = &["Clean", "Warm"];
///
///     fn from_index(index: usize) -> Self {
///         match index {
///             0 => Mode::Clean,
///             _ => Mode::Warm,
///         }
///     }
///
///     fn index(self) -> usize {
///         self as usize
///     }
/// }
///
/// const MODE: ChoiceDecl<Mode> = ChoiceDecl::new("mode", "Mode", Mode::Warm).checked();
///
/// struct Drive {
///     mode: Choice<Mode>,
/// }
///
/// impl Params for Drive {
///     fn declare(p: &mut Declarer) -> Self {
///         Drive { mode: p.add(MODE) }
///     }
/// }
///
/// let (_, params) = ParamSet::declare::<Drive>()?;
/// assert_eq!(params.mode.get(), Mode::Warm);
/// # Ok::<(), seam_params::DeclError>(())
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChoiceDecl<E> {
    key: &'static str,
    name: &'static str,
    default: E,
    group: &'static str,
}

impl<E: Choices> ChoiceDecl<E> {
    /// Declares one of `E`'s values that starts at `default`, under `key`,
    /// shown as `name`, at the top level.
    pub const fn new(key: &'static str, name: &'static str, default: E) -> Self {
        ChoiceDecl {
            key,
            name,
            default,
            group: "",
        }
    }

    /// Puts it in a group, as [`FloatDecl::with_group`] does.
    #[must_use]
    pub const fn with_group(mut self, group: &'static str) -> Self {
        self.group = group;
        self
    }

    /// Hands the declaration back, or panics on its key, its group or `E`'s
    /// labels, as [`FloatDecl::checked`] does. [`ChoiceDecl`] shows one.
    ///
    /// # Panics
    ///
    /// On the first rule the declaration breaks, with that rule in words.
    #[must_use]
    pub const fn checked(self) -> Self {
        match self.check() {
            Ok(()) => self,
            Err(error) => panic!("{}", error.rule()),
        }
    }

    /// The first rule it breaks on its own, short of its default.
    const fn check(&self) -> Result<(), DeclError> {
        rules::and(
            rules::and(rules::key(self.key), rules::group(self.key, self.group)),
            rules::labels(self.key, E::LABELS),
        )
    }
}

/// A typed declaration, [`FloatDecl`], [`BoolDecl`] or [`ChoiceDecl`], and
/// the kind of parameter [`Declarer::add`] makes from it.
///
/// It is sealed: only this crate's declarations implement it.
///
/// # Examples
///
/// ```
/// use seam_params::{Bool, BoolDecl, Declaration, Declarer, Float, FloatDecl, ParamSet, Params};
///
/// /// Declares a parameter of any kind, in a build that has a sidechain.
/// fn sidechain<D: Declaration>(p: &mut Declarer, decl: D) -> Option<D::Param> {
///     cfg!(feature = "sidechain").then(|| p.add(decl))
/// }
///
/// struct Compressor {
///     threshold: Float,
///     listen: Option<Bool>,
/// }
///
/// impl Params for Compressor {
///     fn declare(p: &mut Declarer) -> Self {
///         Compressor {
///             threshold: p.add(FloatDecl::new("threshold", "Threshold", -60.0..=0.0, -20.0)),
///             listen: sidechain(p, BoolDecl::new("listen", "Listen", false)),
///         }
///     }
/// }
///
/// let (set, params) = ParamSet::declare::<Compressor>()?;
/// assert!(params.listen.is_none()); // this doc test has no such feature
/// assert_eq!(set.len(), 1);
/// # Ok::<(), seam_params::DeclError>(())
/// ```
pub trait Declaration: Sealed {
    /// The typed parameter it makes.
    type Param;

    /// Declares it through `p`.
    #[doc(hidden)]
    fn declare(self, p: &mut Declarer, token: Token) -> Self::Param;
}

impl Sealed for FloatDecl {}
impl Sealed for BoolDecl {}
impl<E> Sealed for ChoiceDecl<E> {}

/// The range a refused float's lone store is made with, since its own may be
/// one no range can be made from. The parameter is in no set, and the build
/// that refused it drops what `declare` returned.
const STAND_IN: Kind = Kind::continuous(0.0, 1.0, 0.0, None);

impl Declaration for FloatDecl {
    type Param = Float;

    fn declare(self, p: &mut Declarer, _: Token) -> Float {
        let (kind, refused) = match self.check() {
            Ok(range) => (Kind::Continuous(range), None),
            Err(error) => (STAND_IN, Some(error)),
        };
        let decl = ParamDecl::new(self.key, self.name, kind, self.default)
            .with_unit(self.unit)
            .with_group(self.group);
        Float(p.store(decl, refused))
    }
}

impl Declaration for BoolDecl {
    type Param = Bool;

    fn declare(self, p: &mut Declarer, _: Token) -> Bool {
        let default = if self.default { 1.0 } else { 0.0 };
        let decl = ParamDecl::new(self.key, self.name, Kind::Bool, default).with_group(self.group);
        let decl = if self.bypass { decl.as_bypass() } else { decl };
        Bool(p.store(decl, self.check().err()))
    }
}

impl<E: Choices> Declaration for ChoiceDecl<E> {
    type Param = Choice<E>;

    fn declare(self, p: &mut Declarer, _: Token) -> Choice<E> {
        let index = self.default.index();
        let refused = self.check().err().or_else(|| {
            (index >= E::LABELS.len()).then_some(DeclError::DefaultOutsideRange(self.key))
        });
        let decl = ParamDecl::new(self.key, self.name, Kind::Choice(E::LABELS), index as f32)
            .with_group(self.group);
        Choice(p.store(decl, refused), PhantomData)
    }
}

/// A typed parameter, [`Float`], [`Bool`] or [`Choice`], and the declaration
/// that makes it: [`FloatDecl`], [`BoolDecl`] or [`ChoiceDecl`].
///
/// `#[derive(Params)]` reads each field's declaration from its type through
/// this trait, never from the type's name, so a type alias works there. It
/// is sealed: only this crate's parameters implement it.
///
/// # Examples
///
/// What the derive writes for a field whose type is an alias:
///
/// ```
/// use seam_params::{Declared, Float, FloatDecl};
///
/// type Decibels = Float;
///
/// const GAIN: <Decibels as Declared>::Decl =
///     FloatDecl::new("gain", "Gain", -60.0..=12.0, 0.0).checked();
/// ```
pub trait Declared: Sealed + Sized {
    /// The declaration that makes this parameter.
    type Decl: Declaration<Param = Self>;
}

impl Declared for Float {
    type Decl = FloatDecl;
}

impl Declared for Bool {
    type Decl = BoolDecl;
}

impl<E: Choices> Declared for Choice<E> {
    type Decl = ChoiceDecl<E>;
}

/// Panics, at compile time in `#[derive(Params)]`, when `key` is one of
/// `earlier`, the keys of the fields before it in the struct.
///
/// # Panics
///
/// For such a key, which is the point.
#[doc(hidden)]
pub const fn key_is_new(key: &'static str, earlier: &[&'static str]) {
    let mut at = 0;
    while at < earlier.len() {
        if rules::same(earlier[at].as_bytes(), key.as_bytes()) {
            panic!("{}", DeclError::DuplicateKey(key).rule());
        }
        at += 1;
    }
}

/// A struct of parameters that declares each one and keeps what it is
/// handed.
///
/// [`declare`](Self::declare) adds each parameter through the [`Declarer`],
/// in the order a host should list it within its group, and returns the
/// struct. It returns no `Result`: a refused declaration is kept, and
/// [`ParamSet::declare`] returns it. A struct may hold another, whose
/// `declare` it calls with the same declarer. `()` is the struct of a plugin
/// with no parameters.
///
/// # Examples
///
/// ```
/// use seam_params::{Bool, BoolDecl, Declarer, Float, FloatDecl, ParamSet, Params};
///
/// struct Band {
///     on: Bool,
///     freq: Float,
/// }
///
/// impl Params for Band {
///     fn declare(p: &mut Declarer) -> Self {
///         Band {
///             on: p.add(BoolDecl::new("low.on", "On", true).with_group("Low")),
///             freq: p.add(
///                 FloatDecl::new("low.freq", "Frequency", 20.0..=500.0, 100.0)
///                     .with_skew_centre(100.0)
///                     .with_unit("Hz")
///                     .with_group("Low"),
///             ),
///         }
///     }
/// }
///
/// struct Equaliser {
///     low: Band,
///     out: Float,
/// }
///
/// impl Params for Equaliser {
///     fn declare(p: &mut Declarer) -> Self {
///         Equaliser {
///             low: Band::declare(p),
///             out: p.add(FloatDecl::new("out", "Output", -24.0..=24.0, 0.0)),
///         }
///     }
/// }
///
/// let (set, eq) = ParamSet::declare::<Equaliser>()?;
/// // Hosts list the top level first, then each group.
/// let keys: Vec<_> = set.ids().map(|id| set.decl(id).key).collect();
/// assert_eq!(keys, ["out", "low.on", "low.freq"]);
/// assert_eq!(eq.low.freq.get(), 100.0);
/// # Ok::<(), seam_params::DeclError>(())
/// ```
pub trait Params: Sized {
    /// Declares every parameter through `p` and returns them.
    fn declare(p: &mut Declarer) -> Self;
}

impl Params for () {
    fn declare(_: &mut Declarer) -> Self {}
}

/// Where a struct's declarations go while [`ParamSet::declare`] builds
/// their set.
///
/// It keeps the first declaration the set refuses and goes on, so
/// [`Params::declare`] runs to its end. The parameter handed back for that
/// declaration, and for each one after it, has a store of its own and is in
/// no set. The build then returns the refusal, and drops what `declare`
/// returned.
///
/// # Examples
///
/// A refusal midway: `declare` still runs to its end, and the build returns
/// that refusal.
///
/// ```
/// use seam_params::{Bool, BoolDecl, DeclError, Declarer, ParamSet, Params};
///
/// struct Clash {
///     on: Bool,
///     again: Bool,
///     unkeyed: Bool,
/// }
///
/// impl Params for Clash {
///     fn declare(p: &mut Declarer) -> Self {
///         Clash {
///             on: p.add(BoolDecl::new("on", "On", true)),
///             // Refused: the key is taken. `declare` goes on.
///             again: p.add(BoolDecl::new("on", "On again", false)),
///             // Not checked: after a refusal, nothing joins the set.
///             unkeyed: p.add(BoolDecl::new("", "No key", false)),
///         }
///     }
/// }
///
/// let refused = ParamSet::declare::<Clash>().map(|_| ()).unwrap_err();
/// assert_eq!(refused, DeclError::DuplicateKey("on"));
/// ```
#[derive(Debug)]
pub struct Declarer {
    builder: ParamSetBuilder,
    refused: Option<DeclError>,
}

impl Declarer {
    fn new() -> Self {
        Declarer {
            builder: ParamSetBuilder::new(),
            refused: None,
        }
    }

    /// Declares one parameter and hands it back, carrying its store.
    ///
    /// The declaration's type says what kind of parameter it is, so a field
    /// of another kind does not compile. The parameter has no id until the
    /// set is built: ask for one after [`ParamSet::declare`] returns.
    /// [`Params`] shows several declared.
    pub fn add<D: Declaration>(&mut self, decl: D) -> D::Param {
        decl.declare(self, Token(()))
    }

    /// Declares a plain table, in order, as
    /// [`ParamSet::from_decls`] would.
    ///
    /// For a test plugin, or one whose parameters come from data. Nothing is
    /// handed back: [`ParamSet::bind`] makes a typed parameter from a key
    /// once the set is built.
    ///
    /// # Examples
    ///
    /// ```
    /// use seam_params::{Declarer, Float, Kind, ParamDecl, ParamSet, Params};
    ///
    /// const TABLE: &[ParamDecl] = &[
    ///     ParamDecl::new("a", "A", Kind::Bool, 0.0),
    ///     ParamDecl::new("b", "B", Kind::continuous(0.0, 1.0, 0.0, None), 0.5),
    /// ];
    ///
    /// struct FromTable;
    ///
    /// impl Params for FromTable {
    ///     fn declare(p: &mut Declarer) -> Self {
    ///         p.extend(TABLE.iter().copied());
    ///         FromTable
    ///     }
    /// }
    ///
    /// let (set, _) = ParamSet::declare::<FromTable>()?;
    /// let b: Float = set.bind("b")?;
    /// assert_eq!(b.get(), 0.5);
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    pub fn extend(&mut self, decls: impl IntoIterator<Item = ParamDecl>) {
        for decl in decls {
            if self.refused.is_some() {
                return;
            }
            if let Err(error) = self.builder.add(decl) {
                self.refused = Some(error);
            }
        }
    }

    /// The store for `decl`: in the set when it and every declaration before
    /// it were accepted, and alone otherwise. `refused` is what the typed
    /// declaration found wrong with itself.
    fn store(&mut self, decl: ParamDecl, refused: Option<DeclError>) -> Arc<ParamCell> {
        if self.refused.is_none() {
            let added = match refused {
                None => self.builder.add_cell(decl),
                Some(error) => Err(error),
            };
            match added {
                Ok(cell) => return cell,
                Err(error) => self.refused = Some(error),
            }
        }
        Arc::new(ParamCell::new(decl))
    }
}

impl ParamSet {
    /// Declares a struct of parameters and freezes the set they are in,
    /// handing back both.
    ///
    /// This is how a plugin makes its set: each parameter is declared where
    /// it is held, and carries its store. [`from_decls`](Self::from_decls)
    /// makes one from a plain table.
    ///
    /// # Errors
    ///
    /// Returns the [`DeclError`] of the first declaration refused, which may
    /// be one only the whole set shows, such as a key two parameters share.
    /// Nothing the struct's [`declare`](Params::declare) returned is kept.
    ///
    /// # Examples
    ///
    /// ```
    /// use seam_params::{Bool, BoolDecl, DeclError, Declarer, ParamSet, Params};
    ///
    /// struct Twice {
    ///     first: Bool,
    ///     second: Bool,
    /// }
    ///
    /// impl Params for Twice {
    ///     fn declare(p: &mut Declarer) -> Self {
    ///         Twice {
    ///             first: p.add(BoolDecl::new("on", "On", true)),
    ///             second: p.add(BoolDecl::new("on", "On again", false)),
    ///         }
    ///     }
    /// }
    ///
    /// let refused = ParamSet::declare::<Twice>().map(|_| ()).unwrap_err();
    /// assert_eq!(refused, DeclError::DuplicateKey("on"));
    /// ```
    pub fn declare<P: Params>() -> Result<(Arc<ParamSet>, P), DeclError> {
        let mut declarer = Declarer::new();
        let params = P::declare(&mut declarer);
        match declarer.refused {
            Some(error) => Err(error),
            None => Ok((declarer.builder.build(), params)),
        }
    }
}
