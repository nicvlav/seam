//! Typed parameters: binding checks each against its declaration once, each
//! carries its store and reads as its own type, and a group check tells the
//! audio thread when any of its parameters moved.

use std::sync::Arc;

use seam_params::{
    BindError, Bool, Changes, Choice, Choices, Float, Kind, ParamDecl, ParamId, ParamSet,
    audio_thread,
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

/// The same labels as [`Mode`]'s but one, in the same order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Renamed {
    Clean,
    Warm,
    Loud,
}

impl Choices for Renamed {
    const LABELS: &'static [&'static str] = &["Clean", "Warm", "Loud"];

    fn from_index(index: usize) -> Self {
        match index {
            0 => Renamed::Clean,
            1 => Renamed::Warm,
            _ => Renamed::Loud,
        }
    }

    fn index(self) -> usize {
        self as usize
    }
}

const PARAMS: &[ParamDecl] = &[
    ParamDecl::new(
        "gain",
        "Gain",
        Kind::continuous(-60.0, 12.0, 0.1, None),
        0.0,
    ),
    ParamDecl::new("mode", "Mode", Kind::Choice(Mode::LABELS), 0.0),
    ParamDecl::new("bypass", "Bypass", Kind::Bool, 0.0).as_bypass(),
    ParamDecl::new("mix", "Mix", Kind::continuous(0.0, 100.0, 0.0, None), 100.0),
];

fn set() -> Arc<ParamSet> {
    ParamSet::from_decls(PARAMS.iter().copied()).expect("valid declarations")
}

#[derive(Debug, Clone)]
struct Params {
    gain: Float,
    mode: Choice<Mode>,
    bypass: Bool,
    mix: Float,
}

fn bind(set: &ParamSet) -> Params {
    Params {
        gain: set.bind("gain").expect("a continuous parameter"),
        mode: set.bind("mode").expect("a choice with Mode's labels"),
        bypass: set.bind("bypass").expect("a bool"),
        mix: set.bind("mix").expect("a continuous parameter"),
    }
}

#[test]
fn a_parameter_bound_to_another_kind_is_refused_with_its_key() {
    let set = set();
    assert_eq!(set.bind::<Bool>("gain"), Err(BindError::WrongKind("gain")));
    assert_eq!(
        set.bind::<Float>("bypass"),
        Err(BindError::WrongKind("bypass"))
    );
    assert_eq!(set.bind::<Float>("mode"), Err(BindError::WrongKind("mode")));
    assert_eq!(
        set.bind::<Choice<Mode>>("bypass"),
        Err(BindError::WrongKind("bypass"))
    );
    let error = set
        .bind::<Bool>("gain")
        .expect_err("a continuous parameter");
    assert!(error.to_string().contains(r#""gain""#), "{error}");
}

#[test]
fn a_key_nothing_is_declared_under_is_refused_by_name() {
    let set = set();
    let error = set.bind::<Float>("volume").expect_err("not declared");
    assert_eq!(error, BindError::NotDeclared("volume"));
    assert!(error.to_string().contains(r#""volume""#), "{error}");
    // The display name finds nothing either: only the key binds.
    assert_eq!(
        set.bind::<Float>("Gain"),
        Err(BindError::NotDeclared("Gain"))
    );
}

#[test]
fn a_choice_whose_enum_has_other_labels_is_refused_with_its_key() {
    let set = set();
    let error = set
        .bind::<Choice<Renamed>>("mode")
        .expect_err("one label differs");
    assert_eq!(error, BindError::LabelsDiffer("mode"));
    assert!(error.to_string().contains(r#""mode""#), "{error}");
}

#[test]
fn each_parameter_reads_with_get_what_the_ui_wrote() {
    let set = set();
    let params = bind(&set);
    let ui = set.take_ui().expect("the first UI handle");

    assert!(ui.write(&params.gain, -6.0));
    assert_eq!(params.gain.get(), -6.0);
    for mode in [Mode::Hot, Mode::Clean, Mode::Warm] {
        assert!(ui.write(&params.mode, mode));
        assert_eq!(params.mode.get(), mode);
        assert_eq!(set.value(params.mode.id()), mode.index() as f32);
    }
    assert!(ui.write(&params.bypass, true));
    assert!(params.bypass.get());
    assert!(!ui.write(&params.bypass, true), "no change, so no gesture");
    assert!(ui.write(&params.bypass, false));
    assert!(!params.bypass.get());
}

#[test]
fn a_typed_write_snaps_and_refuses_as_a_write_by_id_does() {
    let set = set();
    let params = bind(&set);
    let ui = set.take_ui().expect("the first UI handle");

    assert!(ui.write(&params.gain, -6.04));
    assert_eq!(params.gain.get(), -6.0);
    assert!(!ui.write(&params.gain, f32::NAN));
    assert!(ui.write(&params.gain, 100.0));
    assert_eq!(params.gain.get(), 12.0);
}

#[test]
fn a_typed_parameter_and_its_id_read_one_store() {
    let set = set();
    let params = bind(&set);
    let ui = set.take_ui().expect("the first UI handle");
    let id: ParamId = set.id("mode").expect("declared");
    assert_eq!(params.mode.id(), id);

    // A write by id is read through the typed parameter...
    set.audio().host_write(id, 1.0);
    assert_eq!(params.mode.get(), Mode::Hot);
    assert!(ui.write(id, 1.0));
    assert_eq!(params.mode.get(), Mode::Warm);
    // ...and a typed write by id.
    assert!(ui.write(&params.mode, Mode::Clean));
    assert_eq!(set.value(id), 0.0);
    assert_eq!(ui.value(id), 0.0);
    assert_eq!(set.read(id).changes, 3);
}

#[test]
fn a_parameter_outlives_its_set_and_still_reads_its_store() {
    let set = set();
    let gain: Float = set.bind("gain").expect("continuous");
    set.audio().host_write(gain.id(), 1.0);
    drop(set);
    assert_eq!(gain.get(), 12.0);
}

#[test]
fn a_clone_is_the_same_parameter_and_two_parameters_are_not() {
    let set = set();
    let params = bind(&set);
    let again = bind(&set);

    assert_eq!(params.gain.clone(), params.gain);
    assert_eq!(again.gain, params.gain, "bound twice, one parameter");
    assert_ne!(params.gain, params.mix);
    assert_eq!(params.mode.clone(), params.mode);
    assert_eq!(params.bypass.clone(), params.bypass);

    // The same key in another set is another parameter.
    let other = bind(&self::set());
    assert_ne!(other.gain, params.gain);
    assert_eq!(other.gain.id(), params.gain.id());
}

#[test]
fn debug_names_the_parameter_and_not_its_value() {
    let set = set();
    let params = bind(&set);
    let shown = format!("{:?}", params.mode);
    assert!(shown.starts_with("Choice"), "{shown}");
    assert!(shown.contains(r#""mode""#), "{shown}");
}

#[cfg(debug_assertions)]
#[test]
fn a_write_through_another_sets_ui_is_a_contract_panic_and_changes_nothing() {
    use seam_testkit::contract::expect_contract_panic;

    let ours = set();
    let theirs = set();
    let gain: Float = theirs.bind("gain").expect("continuous");
    let ui = ours.take_ui().expect("the first UI handle");

    expect_contract_panic("a parameter of another set", || {
        ui.write(&gain, -6.0);
    });
    assert_eq!(gain.get(), 0.0);
    assert_eq!(ours.value(gain.id()), 0.0);
    assert_eq!(ours.read(gain.id()).changes, 0);
}

#[cfg(not(debug_assertions))]
#[test]
fn a_write_through_another_sets_ui_is_refused_and_changes_nothing() {
    let ours = set();
    let theirs = set();
    let gain: Float = theirs.bind("gain").expect("continuous");
    let ui = ours.take_ui().expect("the first UI handle");

    assert!(!ui.write(&gain, -6.0));
    assert_eq!(gain.get(), 0.0);
    assert_eq!(ours.value(gain.id()), 0.0);
    assert_eq!(ours.read(gain.id()).changes, 0);
}

#[test]
fn a_group_moves_once_per_write_to_a_member_and_never_for_another() {
    let set = set();
    let params = bind(&set);
    let audio = set.audio();
    let mut group = Changes::new();
    let mut moved = || group.moved([&params.gain, &params.mode, &params.bypass]);

    assert!(moved(), "the first check");
    assert!(!moved());

    audio.host_write(params.mix.id(), 0.5);
    assert!(!moved(), "a write outside the group");

    for member in [params.gain.id(), params.mode.id(), params.bypass.id()] {
        audio.host_write(member, 1.0);
        assert!(moved());
        assert!(!moved());
        // A write of the value it holds is a write too.
        audio.host_write(member, 1.0);
        assert!(moved());
        assert!(!moved());
    }

    // Writes to two members between checks are one move.
    audio.host_write(params.gain.id(), 0.0);
    audio.host_write(params.bypass.id(), 0.0);
    assert!(moved());
    assert!(!moved());
}

#[test]
fn a_group_asked_about_other_parameters_than_last_time_moves() {
    let set = set();
    let params = bind(&set);
    let other = bind(&self::set());
    let mut group = Changes::new();

    assert!(group.moved([&params.gain, &params.mix]));
    assert!(!group.moved([&params.gain, &params.mix]));
    assert!(group.moved([&params.gain]), "one member fewer");
    assert!(group.moved([&params.mix]), "another member");
    assert!(
        group.moved([&other.mix]),
        "the same key and count in another set"
    );
    assert!(!group.moved([&other.mix]));
    assert!(group.moved([]), "no members");
    assert!(!group.moved([]));
}

#[test]
fn reads_and_the_group_check_run_under_the_audio_thread_mark() {
    let set = set();
    let params = bind(&set);
    let audio = set.audio();
    let mut group = Changes::new();

    let clone = {
        // A lock, or a drop that must stay off the audio thread, panics here
        // in a debug build.
        let _mark = audio_thread::enter();
        assert!(group.moved([&params.gain, &params.mode, &params.bypass]));
        audio.host_write(params.mode.id(), 0.5);
        assert!(group.moved([&params.gain, &params.mode, &params.bypass]));
        assert_eq!(params.gain.get(), 0.0);
        assert_eq!(params.mode.get(), Mode::Warm);
        assert!(!params.bypass.get());
        // A clone is an atomic increment, allowed here; its drop is not.
        params.gain.clone()
    };
    drop(clone);
}

#[cfg(debug_assertions)]
#[test]
fn dropping_a_typed_parameter_under_the_audio_thread_mark_is_a_contract_panic() {
    use seam_testkit::contract::expect_contract_panic;

    let set = set();
    let params = bind(&set);
    let (gain, mode, bypass) = (
        params.gain.clone(),
        params.mode.clone(),
        params.bypass.clone(),
    );

    let _mark = audio_thread::enter();
    expect_contract_panic("dropping a Float must not run on the audio thread", || {
        drop(gain);
    });
    expect_contract_panic("dropping a Choice must not run on the audio thread", || {
        drop(mode);
    });
    expect_contract_panic("dropping a Bool must not run on the audio thread", || {
        drop(bypass);
    });
}

/// A choice of an enum that is neither `Send` nor `Sync`.
#[derive(Debug, Clone, Copy)]
struct Pointer {
    _not_send: std::marker::PhantomData<*const ()>,
}

impl Choices for Pointer {
    const LABELS: &'static [&'static str] = &["A", "B"];

    fn from_index(_: usize) -> Self {
        Pointer {
            _not_send: std::marker::PhantomData,
        }
    }

    fn index(self) -> usize {
        0
    }
}

#[test]
fn typed_parameters_and_a_group_check_cross_threads_whatever_the_enum() {
    fn send_and_sync<T: Send + Sync>() {}
    send_and_sync::<Float>();
    send_and_sync::<Bool>();
    send_and_sync::<Choice<Pointer>>();
    send_and_sync::<Changes>();
}
