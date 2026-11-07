//! Replay: the baseline follows it, it never records itself, and stale steps
//! replay as nothing.

use std::sync::Arc;

use proptest::prelude::*;
use proptest::test_runner::FileFailurePersistence;
use seam_params::{
    HostEvent, Kind, ParamDecl, ParamId, ParamSet, ParamSetBuilder, UiParams, Value,
};
use seam_undo::{Bound, History};

struct Rig {
    set: Arc<ParamSet>,
    ui: UiParams,
    history: History,
    ids: Vec<ParamId>,
}

fn rig() -> Rig {
    let mut builder = ParamSetBuilder::new();
    for key in ["a", "b"] {
        builder
            .add(ParamDecl::new(
                key,
                key,
                Kind::continuous(0.0, 10.0, 1.0, None),
                5.0,
            ))
            .expect("valid");
    }
    let set = builder.build();
    let ui = set.take_ui().expect("the first UI handle");
    let history = History::new(&ui, Bound::default()).expect("the first history");
    history.enroll_params();
    let ids = set.ids().collect();
    Rig {
        set,
        ui,
        history,
        ids,
    }
}

#[test]
fn a_parameter_replay_is_one_complete_host_gesture_and_a_no_op_is_nothing() {
    // A replay brackets a complete host gesture.
    let rig = rig();
    let (a, b) = (rig.ids[0], rig.ids[1]);
    let mut host = rig.set.take_host_event_drain().expect("the first drain");
    assert!(rig.ui.write(a, 8.0));
    assert!(rig.ui.write(b, 2.0));
    rig.history.tick();
    host.drain(|_| {});

    rig.history.undo().expect("idle");
    let mut events = Vec::new();
    host.drain(|event| events.push(event));
    assert_eq!(
        events,
        [
            HostEvent::Begin(b),
            HostEvent::Value {
                id: b,
                normalized: 0.5
            },
            HostEvent::End(b),
        ]
    );

    // The host already moved `a` back to where the undo would put it.
    rig.set.audio().host_write(a, 0.5);
    rig.history.undo().expect("idle");
    let mut events = Vec::new();
    host.drain(|event| events.push(event));
    assert!(
        events.is_empty(),
        "a replay that changes nothing does nothing"
    );
    assert_eq!(rig.history.cursor(), 0);
    host.finish(|_| {});
}

#[test]
fn a_replay_drives_the_value_through_its_normal_path_and_records_nothing() {
    // Every surface follows a replay.
    let rig = rig();
    let a = rig.ids[0];
    let mut watch = rig.ui.watch(a);
    assert!(rig.ui.write(a, 9.0));
    rig.history.tick();
    assert_eq!(watch.poll(), Some(9.0));
    rig.history.undo().expect("idle");
    assert_eq!(watch.poll(), Some(5.0));
    rig.history.tick();
    assert_eq!(rig.history.len(), 1);
    assert_eq!(rig.history.cursor(), 0);
    assert_eq!(
        rig.history.redo(),
        Ok(true),
        "the replay made no record, so the redo survived"
    );
}

/// A change the property records as its own entry.
#[derive(Debug, Clone)]
enum Change {
    Param { param: usize, value: u8 },
    Value { value: u8 },
}

fn change() -> impl Strategy<Value = Change> {
    prop_oneof![
        (0_usize..2, 0_u8..=10).prop_map(|(param, value)| Change::Param { param, value }),
        (0_u8..=200).prop_map(|value| Change::Value { value }),
    ]
}

#[derive(Debug, Clone)]
enum Move {
    Undo,
    Redo,
    Jump(usize),
}

fn moves() -> impl Strategy<Value = Vec<Move>> {
    proptest::collection::vec(
        prop_oneof![
            Just(Move::Undo),
            Just(Move::Redo),
            (0_usize..24).prop_map(Move::Jump),
        ],
        0..40,
    )
}

impl Rig {
    fn state(&self, value: &Value<u8>) -> (f32, f32, u8) {
        (
            self.set.value(self.ids[0]),
            self.set.value(self.ids[1]),
            value.get(),
        )
    }

    /// Applies `change` and ticks; returns whether it recorded an entry.
    fn apply(&self, value: &Value<u8>, change: &Change) -> bool {
        let version = self.history.version();
        match *change {
            Change::Param { param, value } => {
                self.ui.write(self.ids[param], f32::from(value));
            }
            Change::Value { value: next } => value.set(next),
        }
        self.history.tick();
        self.history.version() != version
    }
}

proptest! {
    #![proptest_config(ProptestConfig {
        failure_persistence: Some(Box::new(FileFailurePersistence::Direct(
            "tests/replay.proptest-regressions",
        ))),
        ..ProptestConfig::default()
    })]

    /// Any sequence of undo, redo and jumps leaves the entry count
    /// unchanged, and the state with k entries applied is the same however k
    /// was reached.
    #[test]
    fn replay_is_pure(changes in proptest::collection::vec(change(), 0..20), moves in moves()) {
        let rig = rig();
        let value = Value::new(100_u8, &rig.set);
        rig.history.enroll(&value, "v").expect("first");
        let mut states = vec![rig.state(&value)];
        for change in &changes {
            if rig.apply(&value, change) {
                states.push(rig.state(&value));
            }
        }
        let len = rig.history.len();
        prop_assert_eq!(len + 1, states.len());
        for step in &moves {
            match *step {
                Move::Undo => {
                    rig.history.undo().expect("idle");
                }
                Move::Redo => {
                    rig.history.redo().expect("idle");
                }
                Move::Jump(index) => {
                    let _ = rig.history.jump(index);
                }
            }
            rig.history.tick();
            prop_assert_eq!(rig.history.len(), len);
            prop_assert_eq!(rig.state(&value), states[rig.history.cursor()]);
        }
    }

    /// After undoing, a new change's step starts from the value the undo
    /// left, so undoing it lands there.
    #[test]
    fn an_edit_after_undo_starts_from_the_undone_state(
        changes in proptest::collection::vec(change(), 1..12),
        undos in 1_usize..12,
        edit in change(),
    ) {
        let rig = rig();
        let value = Value::new(100_u8, &rig.set);
        rig.history.enroll(&value, "v").expect("first");
        for change in &changes {
            rig.apply(&value, change);
        }
        for _ in 0..undos {
            rig.history.undo().expect("idle");
        }
        let undone = rig.state(&value);
        let cursor = rig.history.cursor();
        if rig.apply(&value, &edit) {
            prop_assert_eq!(rig.history.len(), cursor + 1, "the edit dropped the redo");
            prop_assert_eq!(rig.history.undo(), Ok(true));
            prop_assert_eq!(rig.state(&value), undone);
        } else {
            prop_assert_eq!(rig.state(&value), undone, "a no-op changed nothing");
        }
    }
}

#[cfg(debug_assertions)]
#[test]
#[should_panic(expected = "holding off loads must not run on the audio thread")]
fn a_replay_that_writes_parameters_is_refused_on_the_audio_thread() {
    let rig = rig();
    assert!(rig.ui.write(rig.ids[0], 8.0));
    rig.history.tick();
    let _audio = seam_params::audio_thread::enter();
    let _ = rig.history.undo();
}
