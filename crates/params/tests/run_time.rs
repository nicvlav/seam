//! Parameters made at run time through the UI handle: the calls that name
//! one parameter take a typed one, listed or made at run time, with a value
//! of its type; a gesture on one made at run time has no id; and the host
//! hears nothing of it.

use seam_params::{
    Bool, BoolDecl, Choice, ChoiceDecl, Choices, Declaration, Declarer, Float, FloatDecl, Origin,
    ParamSet, Params,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    Clean,
    Hot,
}

impl Choices for Mode {
    const LABELS: &'static [&'static str] = &["Clean", "Hot"];

    fn from_index(index: usize) -> Self {
        match index {
            0 => Mode::Clean,
            _ => Mode::Hot,
        }
    }

    fn index(self) -> usize {
        self as usize
    }
}

/// One of each kind, listed.
struct Listed {
    gain: Float,
    on: Bool,
    mode: Choice<Mode>,
}

impl Params for Listed {
    fn declare(p: &mut Declarer) -> Self {
        Listed {
            gain: p.add(FloatDecl::new("gain", "Gain", -60.0..=12.0, 0.0).with_step(0.5)),
            on: p.add(BoolDecl::new("on", "On", false)),
            mode: p.add(ChoiceDecl::new("mode", "Mode", Mode::Clean)),
        }
    }
}

/// The same three, made at run time.
struct Made {
    gain: Float,
    on: Bool,
    mode: Choice<Mode>,
}

impl Params for Made {
    fn declare(p: &mut Declarer) -> Self {
        Made {
            gain: p.add(FloatDecl::new("gain", "Gain", -60.0..=12.0, 0.0).with_step(0.5)),
            on: p.add(BoolDecl::new("on", "On", false)),
            mode: p.add(ChoiceDecl::new("mode", "Mode", Mode::Clean)),
        }
    }
}

fn made(set: &ParamSet) -> Made {
    Declarer::run_time::<Made>(set, Box::new(|_| None)).0
}

fn one<D: Declaration>(set: &ParamSet, decl: D) -> D::Param {
    Declarer::run_time_one(set, Box::new(|_| None), decl).0
}

#[test]
fn a_typed_value_reaches_both_kinds_through_each_call() {
    let (set, listed) = ParamSet::declare::<Listed>().expect("valid");
    let made = made(&set);
    let ui = set.take_ui().expect("the first UI handle");

    // `would_change` with a value of the parameter's own type.
    assert!(ui.would_change(&listed.on, true));
    assert!(ui.would_change(&made.on, true));
    assert!(!ui.would_change(&made.mode, Mode::Clean), "it holds Clean");
    assert!(ui.would_change(&made.gain, -6.2));
    assert!(!ui.would_change(&made.gain, 0.2), "snaps to the 0 it holds");

    // `begin`, `is_open` and `shown`, the value shown of its own type.
    for gain in [&listed.gain, &made.gain] {
        assert!(!ui.is_open(gain));
        let drag = ui.begin(gain);
        assert!(ui.is_open(gain));
        assert!(drag.write(-6.2));
        assert_eq!(ui.shown(gain), Some(-6.0));
        drop(drag);
        assert!(!ui.is_open(gain));
        assert_eq!(ui.shown(gain), None);
        assert_eq!(gain.get(), -6.0);
    }
    let drag = ui.begin(&made.mode);
    assert!(drag.write(1.0), "a gesture writes the stored index");
    assert_eq!(ui.shown(&made.mode), Some(Mode::Hot));
    drop(drag);

    // `Edit::write` takes the type too, and joins one gesture per parameter.
    let mut edit = ui.edit();
    assert!(edit.write(&made.on, true));
    assert!(edit.write(&listed.mode, Mode::Hot));
    assert!(!edit.write(&made.on, true), "no change");
    assert!(ui.is_open(&made.on));
    assert!(!ui.user_gesture_open(), "an edit is not a user's gesture");
    drop(edit);
    assert!(!ui.is_open(&made.on));
    assert!(made.on.get());
    assert_eq!(listed.mode.get(), Mode::Hot);
}

#[test]
fn a_gesture_on_a_parameter_made_at_run_time_has_no_id_and_tells_the_host_nothing() {
    let (set, listed) = ParamSet::declare::<Listed>().expect("valid");
    let made = made(&set);
    let ui = set.take_ui().expect("the first UI handle");
    let mut drain = set.take_host_event_drain().expect("the first drain");

    let drag = ui.begin(&made.gain);
    assert_eq!(drag.id(), None);
    drag.write(-12.0);
    drop(drag);
    assert!(ui.write(&made.on, true));
    let mut heard = Vec::new();
    drain.drain(|event| heard.push(event));
    assert!(heard.is_empty(), "{heard:?}");

    let drag = ui.begin(&listed.gain);
    assert_eq!(drag.id(), Some(listed.gain.id()));
    drag.write(-12.0);
    drop(drag);
    drain.drain(|event| heard.push(event));
    assert_eq!(heard.len(), 3, "a listed one is heard: {heard:?}");
}

#[test]
fn a_parameter_made_at_run_time_is_born_holding_its_held_value() {
    let set = ParamSet::from_decls([]).expect("empty");
    let held = |key: &str| match key {
        "gain" => Some(-7.3),
        "on" => Some(1.0),
        _ => None,
    };
    let (made, erased) = Declarer::run_time::<Made>(&set, Box::new(held));
    assert_eq!(made.gain.get(), -7.5, "made legal as a load's value is");
    assert!(made.on.get());
    assert_eq!(made.mode.get(), Mode::Clean, "nothing held: its default");

    let keys: Vec<_> = erased.iter().map(|param| param.decl().key).collect();
    assert_eq!(keys, ["gain", "on", "mode"], "in declaration order");
    for param in &erased {
        let reading = param.read();
        assert_eq!((reading.changes, reading.origin), (0, Origin::Restore));
        assert!(param.listed().is_none());
        assert!(param.made_by(&set));
    }
    assert_eq!(set.len(), 0, "in no set");
}

#[test]
fn the_plugin_holds_a_parameter_until_its_last_typed_one_drops() {
    let set = ParamSet::from_decls([]).expect("empty");
    let (made, erased) = Declarer::run_time::<Made>(&set, Box::new(|_| None));
    let gain = &erased[0];
    let copy = made.gain.clone();
    drop(made);
    assert!(gain.held(), "a clone holds it");
    drop(copy);
    assert!(!gain.held());
    assert!(gain.downgrade().upgrade().is_some(), "the store lives on");
}

#[cfg(debug_assertions)]
#[test]
fn the_id_of_a_parameter_made_at_run_time_is_a_contract_panic_naming_the_fix() {
    let set = ParamSet::from_decls([]).expect("empty");
    let gain = one(&set, FloatDecl::new("gain", "Gain", -60.0..=12.0, 0.0));
    seam_testkit::contract::expect_contract_panic("pass the parameter itself", || {
        let _ = gain.id();
    });
}

#[cfg(debug_assertions)]
#[test]
fn a_user_write_while_a_restore_applies_names_the_fix_for_a_parameter_made_at_run_time() {
    // A participant applying a restore declares its parameters again; the
    // load restores their values.
    let set = ParamSet::from_decls([]).expect("empty");
    let gain = one(&set, FloatDecl::new("gain", "Gain", -60.0..=12.0, 0.0));
    let ui = set.take_ui().expect("the first UI handle");
    let writer = set.clock().writer();
    let _applying = writer.applying();
    seam_testkit::contract::expect_contract_panic("declare it in the apply instead", || {
        ui.write(&gain, -6.0);
    });
}

#[cfg(debug_assertions)]
#[test]
fn another_instances_parameter_is_a_contract_panic_and_changes_nothing() {
    use seam_params::AnyParam;
    use seam_testkit::contract::expect_contract_panic;

    let ours = ParamSet::from_decls([]).expect("empty");
    let theirs = ParamSet::from_decls([]).expect("empty");
    let gain = one(&theirs, FloatDecl::new("gain", "Gain", -60.0..=12.0, 0.0));
    let ui = ours.take_ui().expect("the first UI handle");

    expect_contract_panic("a parameter of another set", || {
        ui.write(&gain, -6.0);
    });
    expect_contract_panic("a parameter of another set", || {
        let _drag = ui.begin(&gain);
    });
    assert_eq!(gain.get(), 0.0);
    assert_eq!(gain.erased().read().changes, 0);
}

#[cfg(not(debug_assertions))]
#[test]
fn another_instances_parameter_is_refused_and_changes_nothing() {
    let ours = ParamSet::from_decls([]).expect("empty");
    let theirs = ParamSet::from_decls([]).expect("empty");
    let gain = one(&theirs, FloatDecl::new("gain", "Gain", -60.0..=12.0, 0.0));
    let ui = ours.take_ui().expect("the first UI handle");

    assert!(!ui.write(&gain, -6.0));
    let drag = ui.begin(&gain);
    assert_eq!(drag.id(), None);
    assert!(!drag.write(-6.0), "a gesture that writes nothing");
    drop(drag);
    assert!(!ui.would_change(&gain, -6.0));
    assert!(!ui.user_gesture_open());
    assert_eq!(gain.get(), 0.0);
}

#[test]
fn a_mistake_in_a_struct_made_at_run_time_panics_with_the_fix() {
    use seam_params::{Kind, ParamDecl};
    use seam_testkit::contract::expect_contract_panic;

    struct Twice(
        #[expect(dead_code, reason = "made for the panic")] Bool,
        #[expect(dead_code, reason = "made for the panic")] Bool,
    );
    impl Params for Twice {
        fn declare(p: &mut Declarer) -> Self {
            Twice(
                p.add(BoolDecl::new("on", "On", false)),
                p.add(BoolDecl::new("on", "On", true)),
            )
        }
    }
    struct Bypass(#[expect(dead_code, reason = "made for the panic")] Bool);
    impl Params for Bypass {
        fn declare(p: &mut Declarer) -> Self {
            Bypass(p.add(BoolDecl::new("bypass", "Bypass", false).as_bypass()))
        }
    }
    struct Table;
    impl Params for Table {
        fn declare(p: &mut Declarer) -> Self {
            p.extend([ParamDecl::new("on", "On", Kind::Bool, 0.0)]);
            Table
        }
    }
    let set = ParamSet::from_decls([]).expect("empty");

    let message = expect_contract_panic("give each its own", || {
        let _ = Declarer::run_time::<Twice>(&set, Box::new(|_| None));
    });
    assert!(message.contains("\"on\""), "{message}");
    expect_contract_panic("declare the bypass in the plugin's Params", || {
        let _ = Declarer::run_time::<Bypass>(&set, Box::new(|_| None));
    });
    expect_contract_panic("declare each with Declarer::add", || {
        let _ = Declarer::run_time::<Table>(&set, Box::new(|_| None));
    });
    expect_contract_panic("was refused", || {
        let _ = one(&set, FloatDecl::new("", "No key", 0.0..=1.0, 0.0));
    });
}
