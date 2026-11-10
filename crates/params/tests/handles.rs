//! Typed handles: binding checks each against its declaration once, reads and
//! writes carry the handle's type, and a group check tells the audio thread
//! when any of its parameters moved.

use std::sync::Arc;

use proptest::prelude::*;
use proptest::test_runner::FileFailurePersistence;
use seam_params::{
    BindError, Bool, Changes, Choice, Choices, Float, Handle, Kind, ParamDecl, ParamId, ParamSet,
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

struct Handles {
    gain: Float,
    mode: Choice<Mode>,
    bypass: Bool,
    mix: Float,
}

fn bind(set: &ParamSet) -> Handles {
    Handles {
        gain: set.bind("gain").expect("a continuous parameter"),
        mode: set.bind("mode").expect("a choice with Mode's labels"),
        bypass: set.bind("bypass").expect("a bool"),
        mix: set.bind("mix").expect("a continuous parameter"),
    }
}

#[test]
fn a_handle_bound_to_another_kind_is_refused_with_its_key() {
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
fn each_handle_reads_back_through_the_audio_side_what_the_ui_wrote() {
    let set = set();
    let handles = bind(&set);
    let ui = set.take_ui().expect("the first UI handle");
    let audio = set.audio();

    assert!(ui.write(handles.gain, -6.0));
    assert_eq!(audio.value(handles.gain), -6.0);
    for mode in [Mode::Hot, Mode::Clean, Mode::Warm] {
        assert!(ui.write(handles.mode, mode));
        assert_eq!(audio.value(handles.mode), mode);
        assert_eq!(audio.value(handles.mode.id()), mode.index() as f32);
    }
    assert!(ui.write(handles.bypass, true));
    assert!(audio.value(handles.bypass));
    assert!(!ui.write(handles.bypass, true), "no change, so no gesture");
    assert!(ui.write(handles.bypass, false));
    assert!(!audio.value(handles.bypass));
    assert_eq!(ui.value(handles.bypass), set.value(handles.bypass));
}

#[test]
fn a_typed_write_snaps_and_refuses_as_the_untyped_one_does() {
    let set = set();
    let handles = bind(&set);
    let ui = set.take_ui().expect("the first UI handle");

    assert!(ui.write(handles.gain, -6.04));
    assert_eq!(ui.value(handles.gain), -6.0);
    assert!(!ui.write(handles.gain, f32::NAN));
    assert!(ui.write(handles.gain, 100.0));
    assert_eq!(ui.value(handles.gain), 12.0);
}

#[test]
fn a_parameter_id_is_the_untyped_handle() {
    let set = set();
    let id: ParamId = set.id("mode").expect("declared");
    let mode: Choice<Mode> = set.bind("mode").expect("Mode's labels");
    assert_eq!(mode.id(), id);
    assert_eq!(id.id(), id);
    set.audio().host_write(id, 1.0);
    assert_eq!(set.value(id), 2.0);
    assert_eq!(set.value(mode), Mode::Hot);
}

#[test]
fn a_group_moves_once_per_write_to_a_member_and_never_for_another() {
    let set = set();
    let handles = bind(&set);
    let audio = set.audio();
    let mut group: Changes<3> =
        Changes::new([handles.gain.id(), handles.mode.id(), handles.bypass.id()]);

    assert!(group.moved(&audio), "the first check");
    assert!(!group.moved(&audio));

    audio.host_write(handles.mix.id(), 0.5);
    assert!(!group.moved(&audio), "a write outside the group");

    for member in [handles.gain.id(), handles.mode.id(), handles.bypass.id()] {
        audio.host_write(member, 1.0);
        assert!(group.moved(&audio));
        assert!(!group.moved(&audio));
        // A write of the value it holds is a write too.
        audio.host_write(member, 1.0);
        assert!(group.moved(&audio));
        assert!(!group.moved(&audio));
    }

    // Writes to two members between checks are one move.
    audio.host_write(handles.gain.id(), 0.0);
    audio.host_write(handles.bypass.id(), 0.0);
    assert!(group.moved(&audio));
    assert!(!group.moved(&audio));
}

#[test]
fn a_group_may_hold_fewer_than_its_room() {
    let set = set();
    let handles = bind(&set);
    let audio = set.audio();
    let mut group: Changes<4> = Changes::new([handles.mix.id()]);
    let mut empty: Changes<4> = Changes::new([]);

    assert!(group.moved(&audio));
    assert!(empty.moved(&audio));
    audio.host_write(handles.gain.id(), 0.5);
    assert!(!group.moved(&audio));
    audio.host_write(handles.mix.id(), 0.5);
    assert!(group.moved(&audio));
    assert!(!empty.moved(&audio));
}

#[test]
#[should_panic(expected = "a Changes<1> holds at most 1 parameters")]
fn a_group_larger_than_its_room_is_refused() {
    let set = set();
    let handles = bind(&set);
    let _group: Changes<1> = Changes::new([handles.gain.id(), handles.mix.id()]);
}

#[test]
fn the_group_check_and_typed_reads_run_under_the_audio_thread_mark() {
    let set = set();
    let handles = bind(&set);
    let audio = set.audio();
    let mut group: Changes<3> =
        Changes::new([handles.gain.id(), handles.mode.id(), handles.bypass.id()]);

    // A lock, or a drop that must stay off the audio thread, panics here in
    // a debug build.
    let _mark = audio_thread::enter();
    assert!(group.moved(&audio));
    audio.host_write(handles.mode.id(), 0.5);
    assert!(group.moved(&audio));
    assert_eq!(audio.value(handles.gain), 0.0);
    assert_eq!(audio.value(handles.mode), Mode::Warm);
    assert!(!audio.value(handles.bypass));
}

proptest! {
    // Integration tests have no lib.rs for proptest to sit a regressions
    // folder beside, so name the file.
    #![proptest_config(ProptestConfig {
        failure_persistence: Some(Box::new(FileFailurePersistence::Direct(
            "tests/handles.proptest-regressions",
        ))),
        ..ProptestConfig::default()
    })]

    /// Decoding gives a value for every `f32`, each NaN and both infinities
    /// included, and on a stored value it is the inverse of encoding.
    #[test]
    fn decoding_is_total_and_inverts_encoding_on_stored_values(bits in any::<u32>()) {
        let set = set();
        let handles = bind(&set);
        let value = f32::from_bits(bits);

        let on = handles.bypass.decode(value);
        prop_assert_eq!(on, value >= 0.5);
        let mode = handles.mode.decode(value);
        prop_assert!(mode.index() < Mode::LABELS.len());

        // What a store holds: the snapped value, or the start for NaN.
        let stored = Kind::Bool.snap(value).unwrap_or(0.0);
        let again = handles.bypass.encode(handles.bypass.decode(stored));
        prop_assert_eq!(again.to_bits(), stored.to_bits());
        let stored = Kind::Choice(Mode::LABELS).snap(value).unwrap_or(0.0);
        let again = handles.mode.encode(handles.mode.decode(stored));
        prop_assert_eq!(again.to_bits(), stored.to_bits());
    }
}
