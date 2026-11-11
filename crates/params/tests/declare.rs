//! Declaring a struct of parameters: each typed declaration makes its kind of
//! parameter and hands it back, the set lists them as a builder would, and a
//! refused declaration is returned once, when the set is built.

use std::cell::{Cell, RefCell};
use std::panic::{AssertUnwindSafe, catch_unwind};

use seam_params::{
    Bool, BoolDecl, Changes, Choice, ChoiceDecl, Choices, DeclError, Declarer, Float, FloatDecl,
    Kind, ParamDecl, ParamSet, ParamSetBuilder, Params, RangeError,
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

/// Label lists that break a rule, one per `Of<N>`.
const LABEL_SETS: [&[&str]; 4] = [&["Only"], &[" ", "b"], &["a", "b "], &["Tape", "TAPE"]];

/// A choice whose labels are `LABEL_SETS[N]`.
#[derive(Debug, Clone, Copy)]
struct Of<const N: usize>;

impl<const N: usize> Choices for Of<N> {
    const LABELS: &'static [&'static str] = LABEL_SETS[N];

    fn from_index(_: usize) -> Self {
        Of
    }

    fn index(self) -> usize {
        0
    }
}

/// A choice whose value says it sits past the last label.
#[derive(Debug, Clone, Copy)]
struct Beyond;

impl Choices for Beyond {
    const LABELS: &'static [&'static str] = &["a", "b"];

    fn from_index(_: usize) -> Self {
        Beyond
    }

    fn index(self) -> usize {
        2
    }
}

/// One of every kind and option a typed declaration has.
struct Every {
    gain: Float,
    cutoff: Float,
    on: Bool,
    bypass: Bool,
    mode: Choice<Mode>,
}

impl Params for Every {
    fn declare(p: &mut Declarer) -> Self {
        Every {
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
        }
    }
}

#[test]
fn each_declaration_makes_the_parameter_a_person_would_declare_by_hand() {
    let (set, every) = ParamSet::declare::<Every>().expect("valid declarations");

    assert_eq!(
        *set.decl(every.gain.id()),
        ParamDecl::new(
            "gain",
            "Gain",
            Kind::continuous(-60.0, 12.0, 0.1, None),
            -6.0
        )
        .with_unit("dB")
    );
    assert_eq!(
        *set.decl(every.cutoff.id()),
        ParamDecl::new(
            "cutoff",
            "Cutoff",
            Kind::continuous(20.0, 20_000.0, 0.0, Some(1_000.0)),
            1_000.0
        )
        .with_unit("Hz")
        .with_group("Filter")
    );
    assert_eq!(
        *set.decl(every.on.id()),
        ParamDecl::new("on", "On", Kind::Bool, 1.0).with_group("Filter")
    );
    assert_eq!(
        *set.decl(every.bypass.id()),
        ParamDecl::new("bypass", "Bypass", Kind::Bool, 0.0).as_bypass()
    );
    assert_eq!(
        *set.decl(every.mode.id()),
        ParamDecl::new("mode", "Mode", Kind::Choice(Mode::LABELS), 1.0).with_group("Drive")
    );

    // Each starts at its default, read as its own type.
    assert_eq!(every.gain.get(), -6.0);
    assert_eq!(every.cutoff.get(), 1_000.0);
    assert!(every.on.get());
    assert!(!every.bypass.get());
    assert_eq!(every.mode.get(), Mode::Warm);
    assert_eq!(set.bypass(), Some(every.bypass.id()));
}

/// A band of two parameters in a group.
struct Band {
    on: Bool,
    gain: Float,
}

impl Params for Band {
    fn declare(p: &mut Declarer) -> Self {
        Band {
            on: p.add(BoolDecl::new("low.on", "On", true).with_group("Bands/Low")),
            gain: p
                .add(FloatDecl::new("low.gain", "Gain", -24.0..=24.0, 0.0).with_group("Bands/Low")),
        }
    }
}

/// A struct that holds a [`Band`], with parameters around it.
struct Outer {
    mix: Float,
    low: Band,
    out: Float,
    trim: Float,
}

impl Params for Outer {
    fn declare(p: &mut Declarer) -> Self {
        Outer {
            mix: p.add(FloatDecl::new("mix", "Mix", 0.0..=100.0, 100.0).with_group("Out")),
            low: Band::declare(p),
            out: p.add(FloatDecl::new("out", "Output", -24.0..=24.0, 0.0)),
            trim: p.add(FloatDecl::new("trim", "Trim", -6.0..=6.0, 0.0).with_group("Bands")),
        }
    }
}

#[test]
fn a_struct_lists_in_the_builders_order_and_each_id_is_its_keys() {
    let (set, outer) = ParamSet::declare::<Outer>().expect("valid declarations");

    // The same declarations, added to a builder in the order declared.
    let mut builder = ParamSetBuilder::new();
    for (key, group) in [
        ("mix", "Out"),
        ("low.on", "Bands/Low"),
        ("low.gain", "Bands/Low"),
        ("out", ""),
        ("trim", "Bands"),
    ] {
        builder
            .add(ParamDecl::new(key, key, Kind::Bool, 0.0).with_group(group))
            .expect("valid");
    }
    let by_hand = builder.build();
    let keys = |set: &ParamSet| set.ids().map(|id| set.decl(id).key).collect::<Vec<_>>();
    assert_eq!(keys(&set), keys(&by_hand));
    // Groups depth first, a group's own parameters before its subgroups.
    assert_eq!(keys(&set), ["out", "mix", "trim", "low.on", "low.gain"]);

    for (param, key) in [
        (&outer.mix, "mix"),
        (&outer.low.gain, "low.gain"),
        (&outer.out, "out"),
        (&outer.trim, "trim"),
    ] {
        assert_eq!(Some(param.id()), set.id(key), "{key}");
    }
    assert_eq!(Some(outer.low.on.id()), set.id("low.on"));
}

/// Declarations to make, as a [`Run`] makes them.
type Declare = fn(&mut Declarer);

thread_local! {
    /// What [`Run`] declares, set by [`refuses`].
    static DECLARE: Cell<Declare> = Cell::new(|_| {});
}

/// A struct that runs whatever [`DECLARE`] holds.
struct Run;

impl Params for Run {
    fn declare(p: &mut Declarer) -> Self {
        DECLARE.get()(p);
        Run
    }
}

/// Checks that the set refuses each case's declarations with its error.
fn refuses(cases: &[(Declare, DeclError)]) {
    for (declare, expected) in cases {
        DECLARE.set(*declare);
        let refused = ParamSet::declare::<Run>().map(|_| ());
        assert_eq!(refused.err().as_ref(), Some(expected));
    }
}

#[test]
fn a_key_is_refused_by_the_rules_a_table_row_meets() {
    refuses(&[
        (
            |p| _ = p.add(BoolDecl::new("", "B", false)),
            DeclError::EmptyKey,
        ),
        (
            |p| _ = p.add(BoolDecl::new("a/b", "B", false)),
            DeclError::KeyHasSlash("a/b"),
        ),
        (
            |p| {
                _ = p.add(BoolDecl::new(
                    "a_key_that_runs_to_thirty_two_by",
                    "B",
                    false,
                ))
            },
            DeclError::KeyTooLong("a_key_that_runs_to_thirty_two_by"),
        ),
        // "phgz" hashes into the VST3 exporter's range.
        (
            |p| _ = p.add(BoolDecl::new("phgz", "B", false)),
            DeclError::ReservedHostId("phgz"),
        ),
    ]);
}

#[test]
fn what_only_the_whole_set_shows_is_refused_when_it_is_built() {
    refuses(&[
        // "kmtzx" and "k31cd" share one host id.
        (
            |p| {
                _ = p.add(BoolDecl::new("kmtzx", "B", false));
                _ = p.add(BoolDecl::new("k31cd", "B", false));
            },
            DeclError::HostIdCollision {
                key: "k31cd",
                other: "kmtzx",
            },
        ),
        (
            |p| {
                _ = p.add(BoolDecl::new("a", "B", false));
                _ = p.add(ChoiceDecl::new("a", "C", Mode::Clean));
            },
            DeclError::DuplicateKey("a"),
        ),
        (
            |p| {
                _ = p.add(BoolDecl::new("a", "B", false).with_group("fx"));
                _ = p.add(BoolDecl::new("fx", "B", false));
            },
            DeclError::KeyIsGroup("fx"),
        ),
        (
            |p| {
                _ = p.add(BoolDecl::new("fx", "B", false));
                _ = p.add(BoolDecl::new("a", "B", false).with_group("fx"));
            },
            DeclError::KeyIsGroup("fx"),
        ),
        (
            |p| {
                _ = p.add(BoolDecl::new("a", "B", false).as_bypass());
                _ = p.add(BoolDecl::new("b", "B", false).as_bypass());
            },
            DeclError::SecondBypass("b"),
        ),
    ]);
}

#[test]
fn a_group_and_a_choices_labels_are_refused_by_their_rules() {
    refuses(&[
        (
            |p| _ = p.add(BoolDecl::new("fx", "B", false).with_group("Mix/fx")),
            DeclError::KeyIsGroup("fx"),
        ),
        (
            |p| _ = p.add(BoolDecl::new("a", "B", false).with_group("Mix//Sides")),
            DeclError::EmptyGroupName {
                key: "a",
                group: "Mix//Sides",
            },
        ),
        (
            |p| _ = p.add(ChoiceDecl::new("c", "C", Of::<0>)),
            DeclError::TooFewChoices("c"),
        ),
        (
            |p| _ = p.add(ChoiceDecl::new("c", "C", Of::<1>)),
            DeclError::EmptyChoiceLabel("c"),
        ),
        (
            |p| _ = p.add(ChoiceDecl::new("c", "C", Of::<2>)),
            DeclError::PaddedChoiceLabel {
                key: "c",
                label: "b ",
            },
        ),
        (
            |p| _ = p.add(ChoiceDecl::new("c", "C", Of::<3>)),
            DeclError::DuplicateChoiceLabel {
                key: "c",
                label: "TAPE",
            },
        ),
    ]);
}

#[test]
fn a_range_and_a_default_are_refused_by_their_rules() {
    let range = |error| DeclError::RangeRefused { key: "f", error };
    refuses(&[
        (
            |p| _ = p.add(FloatDecl::new("f", "F", f32::NAN..=1.0, 0.0)),
            range(RangeError::NotFinite),
        ),
        (
            |p| _ = p.add(FloatDecl::new("f", "F", 1.0..=0.0, 0.5)),
            range(RangeError::StartNotBelowEnd),
        ),
        (
            |p| _ = p.add(FloatDecl::new("f", "F", 0.0..=1.0, 0.0).with_step(-0.1)),
            range(RangeError::BadInterval),
        ),
        (
            |p| _ = p.add(FloatDecl::new("f", "F", 0.0..=1.0, 0.0).with_skew_centre(1.0)),
            range(RangeError::SkewCentreOutside),
        ),
        (
            |p| _ = p.add(FloatDecl::new("f", "F", 0.0..=1.0, 1.5)),
            DeclError::DefaultOutsideRange("f"),
        ),
        (
            |p| _ = p.add(FloatDecl::new("f", "F", 0.0..=1.0, f32::NAN)),
            DeclError::DefaultOutsideRange("f"),
        ),
        (
            |p| _ = p.add(ChoiceDecl::new("c", "C", Beyond)),
            DeclError::DefaultOutsideRange("c"),
        ),
    ]);
    // Each message names the key, as the table path's do.
    assert_eq!(
        range(RangeError::StartNotBelowEnd).to_string(),
        r#"parameter "f" has a range that is refused: start must be below end"#
    );
    assert_eq!(
        DeclError::DefaultOutsideRange("f").to_string(),
        r#"parameter "f" has a default outside its range"#
    );
}

thread_local! {
    /// What [`Mistakes`] declared, kept past the build that refused it.
    static KEPT: RefCell<Vec<Float>> = const { RefCell::new(Vec::new()) };
}

/// Two mistakes, between parameters the set would take.
struct Mistakes;

impl Params for Mistakes {
    fn declare(p: &mut Declarer) -> Self {
        let declared = [
            p.add(FloatDecl::new("a", "A", 0.0..=1.0, 0.5)),
            p.add(FloatDecl::new("a", "A again", 0.0..=1.0, 0.25)),
            p.add(FloatDecl::new("", "Keyless", 0.0..=1.0, 0.0)),
            p.add(FloatDecl::new("b", "B", 0.0..=1.0, 1.0)),
        ];
        KEPT.with_borrow_mut(|kept| kept.extend(declared));
        Mistakes
    }
}

#[test]
fn the_first_refusal_wins_and_declare_runs_to_its_end() {
    let refused = ParamSet::declare::<Mistakes>().map(|_| ());
    assert_eq!(refused.err(), Some(DeclError::DuplicateKey("a")));

    let kept = KEPT.take();
    assert_eq!(kept.len(), 4, "declare ran to its end");
    // Each was handed a store of its own, at its default, and none has a
    // place in a set: its debug form names no id.
    let defaults: Vec<f32> = kept.iter().map(Float::get).collect();
    assert_eq!(defaults, [0.5, 0.25, 0.0, 1.0]);
    for param in &kept {
        assert!(!format!("{param:?}").contains("id"), "{param:?}");
    }
    #[cfg(debug_assertions)]
    for param in &kept {
        seam_testkit::contract::expect_contract_panic("no id until its set is built", || {
            param.id();
        });
    }
}

/// [`Band`] twice, under one set of keys.
#[expect(dead_code, reason = "the set refuses it, so nothing reads it")]
struct TwoBands {
    first: Band,
    second: Band,
}

impl Params for TwoBands {
    fn declare(p: &mut Declarer) -> Self {
        TwoBands {
            first: Band::declare(p),
            second: Band::declare(p),
        }
    }
}

#[test]
fn one_key_in_two_nested_structs_is_refused_when_the_set_is_built() {
    let refused = ParamSet::declare::<TwoBands>().map(|_| ());
    assert_eq!(refused.err(), Some(DeclError::DuplicateKey("low.on")));
}

#[cfg(debug_assertions)]
struct AsksEarly {
    gain: Float,
}

#[cfg(debug_assertions)]
impl Params for AsksEarly {
    fn declare(p: &mut Declarer) -> Self {
        let gain = p.add(FloatDecl::new("gain", "Gain", -60.0..=12.0, 0.0));
        seam_testkit::contract::expect_contract_panic("no id until its set is built", || {
            gain.id();
        });
        AsksEarly { gain }
    }
}

#[cfg(debug_assertions)]
#[test]
fn asking_for_an_id_inside_declare_is_a_contract_panic() {
    let (set, params) = ParamSet::declare::<AsksEarly>().expect("valid declarations");
    // The set is unharmed, and the id is there once it is built.
    assert_eq!(Some(params.gain.id()), set.id("gain"));
}

const TABLE: &[ParamDecl] = &[
    ParamDecl::new("on", "On", Kind::Bool, 1.0).with_group("Fx"),
    ParamDecl::new(
        "gain",
        "Gain",
        Kind::continuous(-60.0, 12.0, 0.1, None),
        0.0,
    )
    .with_unit("dB"),
    ParamDecl::new("mode", "Mode", Kind::Choice(Mode::LABELS), 2.0),
    ParamDecl::new("bypass", "Bypass", Kind::Bool, 0.0).as_bypass(),
];

/// [`TABLE`], declared through a declarer.
struct Table;

impl Params for Table {
    fn declare(p: &mut Declarer) -> Self {
        p.extend(TABLE.iter().copied());
        Table
    }
}

/// [`TABLE`] with its first row twice.
struct TableTwice;

impl Params for TableTwice {
    fn declare(p: &mut Declarer) -> Self {
        p.extend(TABLE.iter().copied());
        p.extend(TABLE.iter().take(1).copied());
        TableTwice
    }
}

#[test]
fn extend_over_a_table_gives_the_set_from_decls_gives() {
    let (declared, Table) = ParamSet::declare::<Table>().expect("a valid table");
    let from_decls = ParamSet::from_decls(TABLE.iter().copied()).expect("a valid table");
    let decls = |set: &ParamSet| set.ids().map(|id| *set.decl(id)).collect::<Vec<_>>();
    assert_eq!(decls(&declared), decls(&from_decls));
    assert_eq!(declared.bypass(), from_decls.bypass());

    let refused = ParamSet::declare::<TableTwice>().map(|_| ());
    assert_eq!(refused.err(), Some(DeclError::DuplicateKey("on")));
}

#[test]
fn a_struct_of_no_parameters_makes_an_empty_set() {
    let (set, ()) = ParamSet::declare::<()>().expect("nothing to refuse");
    assert!(set.is_empty());
}

#[test]
fn a_declared_parameter_reads_writes_and_reports_changes() {
    let (set, every) = ParamSet::declare::<Every>().expect("valid declarations");
    let ui = set.take_ui().expect("the first UI handle");
    let mut drive = Changes::new();
    assert!(drive.moved([&every.gain, &every.mode]));

    assert!(ui.write(&every.gain, -3.04));
    assert!(ui.write(&every.mode, Mode::Hot));
    assert_eq!(every.gain.get(), -3.0); // snapped to its step
    assert_eq!(every.mode.get(), Mode::Hot);
    assert!(drive.moved([&every.gain, &every.mode]));
    assert!(!drive.moved([&every.gain, &every.mode]));

    // A host write by id reaches the same store.
    set.audio().host_write(every.gain.id(), 1.0);
    assert_eq!(every.gain.get(), 12.0);
    assert!(drive.moved([&every.gain, &every.mode]));
}

/// The message a run-time call of `checked` panics with.
fn panic_message(checked: impl FnOnce()) -> String {
    let payload = catch_unwind(AssertUnwindSafe(checked)).expect_err("checked refuses it");
    payload
        .downcast_ref::<&str>()
        .map(|message| (*message).to_owned())
        .or_else(|| payload.downcast_ref::<String>().cloned())
        .unwrap_or_default()
}

/// A call of `checked`, and the rule it should refuse with.
type Checked = (Box<dyn FnOnce()>, &'static str);

#[test]
fn checked_names_the_rule_a_declaration_breaks() {
    let cases: [Checked; 7] = [
        (
            Box::new(|| _ = FloatDecl::new("f", "F", 1.0..=0.0, 0.5).checked()),
            "start must be below end",
        ),
        (
            Box::new(|| _ = FloatDecl::new("f", "F", 0.0..=1.0, 2.0).checked()),
            "a parameter's default is outside its range",
        ),
        (
            Box::new(|| _ = BoolDecl::new("", "B", false).checked()),
            "a parameter's key is empty",
        ),
        (
            Box::new(|| _ = BoolDecl::new("a/b", "B", false).checked()),
            "a parameter's key contains '/'",
        ),
        (
            Box::new(|| _ = BoolDecl::new("a", "B", false).with_group("/a").checked()),
            "a group path has an empty name in it",
        ),
        (
            Box::new(|| _ = ChoiceDecl::new("c", "C", Of::<0>).checked()),
            "a choice has fewer than two labels",
        ),
        (
            Box::new(|| _ = ChoiceDecl::new("c", "C", Of::<3>).checked()),
            "a choice has two labels that differ only in case",
        ),
    ];
    for (checked, rule) in cases {
        assert_eq!(panic_message(checked), rule);
    }
    // A declaration that breaks no rule is handed back as it was.
    let fine = FloatDecl::new("f", "F", 0.0..=1.0, 0.5).with_step(0.1);
    assert_eq!(fine.checked(), fine);
}
