//! `#[derive(Params)]`: a derived struct declares what the same struct
//! written by hand declares, in the same order, and a struct may hold another
//! of either kind.

use seam_params::{
    Bool, BoolDecl, Choice, ChoiceDecl, Choices, DeclError, Declarer, Float, FloatDecl, ParamDecl,
    ParamSet, Params,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    Clean,
    Warm,
    Hot,
}

impl Choices for Mode {
    const LABELS: &'static [&'static str] = &["Clean", "Warm", "Hot"];

    fn from_index(index: usize) -> Self {
        match index {
            0 => Mode::Clean,
            1 => Mode::Warm,
            _ => Mode::Hot,
        }
    }

    fn index(self) -> usize {
        self as usize
    }
}

/// A constant as an argument, a key and a range alike.
const CUTOFF_KEY: &str = "cutoff";
const TOP: f32 = 20_000.0;

/// A type alias for a parameter type.
type Hertz = Float;

/// The first of two values, behind a call whose brackets hold a comma.
const fn first<A: Copy, B: Copy>(a: A, _: B) -> A {
    a
}

/// Every option and the flag, each kind, an alias, constants and a nested
/// struct.
#[derive(Params)]
struct Derived {
    #[param("gain", "Gain", -60.0..=12.0, first::<f32, f32>(-6.0, 0.0), step = 0.1, unit = "dB")]
    gain: Float,
    #[param(
        CUTOFF_KEY, "Cutoff", 20.0..=TOP, 1_000.0,
        skew_centre = 1_000.0, unit = "Hz", group = "Filter",
    )]
    cutoff: Hertz,
    #[param("on", "On", true, group = "Filter")]
    on: Bool,
    #[param("bypass", "Bypass", false, bypass)]
    bypass: Bool,
    #[param("mode", "Mode", Mode::Warm, group = "Drive")]
    mode: Choice<Mode>,
    band: DerivedBand,
}

#[derive(Params)]
struct DerivedBand {
    #[param("low.on", "On", true, group = "Bands/Low")]
    on: Bool,
}

/// `Derived`, written by hand.
struct ByHand {
    gain: Float,
    cutoff: Float,
    on: Bool,
    bypass: Bool,
    mode: Choice<Mode>,
    band: ByHandBand,
}

impl Params for ByHand {
    fn declare(p: &mut Declarer) -> Self {
        ByHand {
            gain: p.add(
                FloatDecl::new("gain", "Gain", -60.0..=12.0, -6.0)
                    .with_step(0.1)
                    .with_unit("dB"),
            ),
            cutoff: p.add(
                FloatDecl::new("cutoff", "Cutoff", 20.0..=20_000.0, 1_000.0)
                    .with_skew_centre(1_000.0)
                    .with_unit("Hz")
                    .with_group("Filter"),
            ),
            on: p.add(BoolDecl::new("on", "On", true).with_group("Filter")),
            bypass: p.add(BoolDecl::new("bypass", "Bypass", false).as_bypass()),
            mode: p.add(ChoiceDecl::new("mode", "Mode", Mode::Warm).with_group("Drive")),
            band: ByHandBand::declare(p),
        }
    }
}

struct ByHandBand {
    on: Bool,
}

impl Params for ByHandBand {
    fn declare(p: &mut Declarer) -> Self {
        ByHandBand {
            on: p.add(BoolDecl::new("low.on", "On", true).with_group("Bands/Low")),
        }
    }
}

/// Each declaration of `set`, in the order hosts list them.
fn decls(set: &ParamSet) -> Vec<ParamDecl> {
    set.ids().map(|id| *set.decl(id)).collect()
}

#[test]
fn a_derived_struct_declares_what_the_one_written_by_hand_declares() {
    let (derived_set, derived) = ParamSet::declare::<Derived>().expect("valid declarations");
    let (by_hand_set, by_hand) = ParamSet::declare::<ByHand>().expect("valid declarations");

    assert_eq!(decls(&derived_set), decls(&by_hand_set));
    assert_eq!(derived_set.len(), 6);
    assert_eq!(derived_set.bypass(), Some(derived.bypass.id()));
    assert_eq!(by_hand_set.bypass(), Some(by_hand.bypass.id()));

    // Each field holds the parameter its own declaration made.
    let ids = |gain: &Float, cutoff: &Float, on: &Bool, mode: &Choice<Mode>, low: &Bool| {
        [gain.id(), cutoff.id(), on.id(), mode.id(), low.id()]
    };
    assert_eq!(
        ids(
            &derived.gain,
            &derived.cutoff,
            &derived.on,
            &derived.mode,
            &derived.band.on
        ),
        ids(
            &by_hand.gain,
            &by_hand.cutoff,
            &by_hand.on,
            &by_hand.mode,
            &by_hand.band.on
        ),
    );
    assert_eq!(derived.gain.get(), -6.0);
    assert_eq!(derived.cutoff.get(), 1_000.0);
    assert!(derived.on.get());
    assert_eq!(derived.mode.get(), Mode::Warm);
}

/// A derived struct that holds one written by hand.
#[derive(Params)]
struct Mixed {
    #[param("out", "Output", -24.0..=24.0, 0.0)]
    out: Float,
    band: ByHandBand,
}

/// A struct written by hand that holds a derived one.
struct Outer {
    band: DerivedBand,
    out: Float,
}

impl Params for Outer {
    fn declare(p: &mut Declarer) -> Self {
        Outer {
            band: DerivedBand::declare(p),
            out: p.add(FloatDecl::new("out", "Output", -24.0..=24.0, 0.0)),
        }
    }
}

#[test]
fn a_derived_struct_and_one_written_by_hand_hold_each_other() {
    let (mixed_set, mixed) = ParamSet::declare::<Mixed>().expect("valid declarations");
    let (outer_set, outer) = ParamSet::declare::<Outer>().expect("valid declarations");

    assert_eq!(decls(&mixed_set), decls(&outer_set));
    assert_eq!(mixed.out.id(), outer.out.id());
    assert_eq!(mixed.band.on.id(), outer.band.on.id());
}

/// A key that `DerivedBand` has too: only the whole set shows it.
#[derive(Params)]
#[expect(dead_code, reason = "declared only to be refused")]
struct Clash {
    #[param("low.on", "On again", false)]
    again: Bool,
    band: DerivedBand,
}

#[test]
fn a_key_two_structs_share_is_refused_when_the_set_is_built() {
    let refused = ParamSet::declare::<Clash>().map(|_| ()).unwrap_err();
    assert_eq!(refused, DeclError::DuplicateKey("low.on"));
}

#[derive(Params)]
struct Nothing {}

#[test]
fn a_struct_of_no_fields_declares_an_empty_set() {
    let (set, Nothing {}) = ParamSet::declare::<Nothing>().expect("valid declarations");
    assert_eq!(set.len(), 0);
}
