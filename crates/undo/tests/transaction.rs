//! Transactions: one entry per outermost transaction, named by it.

use std::sync::Arc;

use proptest::prelude::*;
use proptest::test_runner::FileFailurePersistence;
use seam_params::{Kind, ParamDecl, ParamId, ParamSet, ParamSetBuilder, UiParams, Value};
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

impl Rig {
    fn names(&self) -> Vec<String> {
        self.history
            .entries()
            .into_iter()
            .map(|entry| entry.name)
            .collect()
    }
}

#[test]
fn every_step_in_a_transaction_joins_one_entry_named_by_the_outermost() {
    // One user operation is one step.
    let rig = rig();
    let (a, b) = (rig.ids[0], rig.ids[1]);
    let sample = Value::new(String::from("kick"), &rig.set);
    rig.history.enroll(&sample, "Sample").expect("first");

    {
        let _outer = rig.history.transaction("Drop sample");
        sample.set("snare".into());
        assert!(rig.ui.write(a, 8.0));
        {
            // Another holder of the history joins the same scope: the depth
            // lives on the history, not per recorder.
            let other = rig.history.clone();
            let _inner = other.transaction("Inner");
            assert!(rig.ui.write(b, 1.0));
            other.tick();
        }
        rig.history.tick();
        sample.set("clap".into());
    }
    assert_eq!(rig.names(), ["Drop sample"]);
    rig.history.undo().expect("idle");
    assert_eq!(
        (rig.set.value(a), rig.set.value(b), sample.get()),
        (5.0, 5.0, "kick".into())
    );
    rig.history.redo().expect("idle");
    assert_eq!(
        (rig.set.value(a), rig.set.value(b), sample.get()),
        (8.0, 1.0, "clap".into())
    );
}

#[test]
fn changes_before_a_transaction_are_their_own_entries_and_an_empty_one_leaves_none() {
    let rig = rig();
    let a = rig.ids[0];
    assert!(rig.ui.write(a, 8.0));
    {
        let _empty = rig.history.transaction("Nothing");
    }
    assert_eq!(rig.names(), ["a"]);
    {
        let _transaction = rig.history.transaction("Set");
        assert!(rig.ui.write(a, 2.0));
    }
    assert_eq!(rig.names(), ["a", "Set"]);
}

#[test]
fn a_transaction_ended_by_a_panic_keeps_what_it_recorded_as_one_entry() {
    let rig = rig();
    let (a, b) = (rig.ids[0], rig.ids[1]);
    let unwound = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _transaction = rig.history.transaction("Load");
        assert!(rig.ui.write(a, 8.0));
        assert!(rig.ui.write(b, 2.0));
        panic!("the load failed");
    }));
    assert!(unwound.is_err());
    assert_eq!(rig.names(), ["Load"]);
    rig.history.undo().expect("idle");
    assert_eq!((rig.set.value(a), rig.set.value(b)), (5.0, 5.0));
}

#[test]
fn an_undo_inside_a_transaction_ends_its_entry_and_the_next_keeps_its_name() {
    // An undo inside a transaction ends its entry, but the next one
    // keeps the transaction's name.
    let rig = rig();
    let (a, b) = (rig.ids[0], rig.ids[1]);
    {
        let _transaction = rig.history.transaction("Randomise");
        assert!(rig.ui.write(a, 8.0));
        rig.history.undo().expect("idle");
        assert_eq!(rig.set.value(a), 5.0);
        assert!(rig.ui.write(b, 3.0));
        assert!(rig.ui.write(a, 1.0));
    }
    assert_eq!(rig.names(), ["Randomise"]);
    assert_eq!(rig.history.cursor(), 1);
    rig.history.undo().expect("idle");
    assert_eq!((rig.set.value(a), rig.set.value(b)), (5.0, 5.0));

    rig.history.redo().expect("idle");
    {
        let _transaction = rig.history.transaction("Morph");
        assert!(rig.ui.write(a, 4.0));
        rig.history.tick();
        rig.history.undo().expect("idle");
        rig.history.redo().expect("idle");
        assert!(rig.ui.write(b, 9.0));
    }
    assert_eq!(rig.names(), ["Randomise", "Morph", "Morph"]);
}

/// One step of a session: changes, transactions opened and closed.
#[derive(Debug, Clone)]
enum Op {
    Open,
    Close,
    Param { param: usize, value: u8 },
    Value { value: u8 },
    Tick,
}

fn op() -> impl Strategy<Value = Op> {
    prop_oneof![
        Just(Op::Open),
        Just(Op::Close),
        (0_usize..2, 0_u8..=10).prop_map(|(param, value)| Op::Param { param, value }),
        (0_u8..=20).prop_map(|value| Op::Value { value }),
        Just(Op::Tick),
    ]
}

proptest! {
    #![proptest_config(ProptestConfig {
        failure_persistence: Some(Box::new(FileFailurePersistence::Direct(
            "tests/transaction.proptest-regressions",
        ))),
        ..ProptestConfig::default()
    })]

    /// One entry per outermost transaction that recorded anything, and per
    /// change outside one; undoing an entry restores every value it
    /// touched.
    #[test]
    fn one_entry_per_outermost_transaction(ops in proptest::collection::vec(op(), 0..50)) {
        let rig = rig();
        let value = Value::new(0_u8, &rig.set);
        rig.history.enroll(&value, "v").expect("first");
        let state = |rig: &Rig| (rig.set.value(rig.ids[0]), rig.set.value(rig.ids[1]), value.get());
        let mut model = Model::default();
        let mut scopes = Vec::new();
        for op in &ops {
            match *op {
                Op::Open => {
                    if scopes.is_empty() {
                        // The outermost open ticks first.
                        model.flush(state(&rig), false);
                        model.scope_changed = false;
                        model.scope_before = state(&rig);
                    }
                    scopes.push(rig.history.transaction("t"));
                }
                Op::Close => {
                    if scopes.len() == 1 {
                        // The outermost close ticks inside the scope.
                        model.flush(state(&rig), true);
                        if model.scope_changed {
                            model.befores.push(model.scope_before);
                        }
                    }
                    scopes.pop();
                }
                Op::Param { param, value } => {
                    let outside = scopes.is_empty();
                    if outside {
                        // One tick records parameter steps before plain
                        // value steps, so the model ticks first to keep
                        // them in the order they happened.
                        rig.history.tick();
                        model.flush(state(&rig), false);
                    }
                    let now = state(&rig);
                    if rig.ui.write(rig.ids[param], f32::from(value)) {
                        if outside {
                            model.befores.push(now);
                            rig.history.tick();
                        } else {
                            model.scope_changed = true;
                        }
                    }
                }
                Op::Value { value: next } => {
                    if model.value_before.is_none() {
                        model.value_before = Some(value.get());
                    }
                    value.set(next);
                }
                Op::Tick => {
                    rig.history.tick();
                    model.flush(state(&rig), !scopes.is_empty());
                }
            }
        }
        while !scopes.is_empty() {
            if scopes.len() == 1 {
                model.flush(state(&rig), true);
                if model.scope_changed {
                    model.befores.push(model.scope_before);
                }
            }
            scopes.pop();
        }
        rig.history.tick();
        model.flush(state(&rig), false);
        prop_assert_eq!(rig.history.len(), model.befores.len());
        for before in model.befores.iter().rev() {
            prop_assert_eq!(rig.history.undo(), Ok(true));
            prop_assert_eq!(state(&rig), *before);
        }
    }
}

/// What the property expects the history to hold.
#[derive(Debug, Default)]
struct Model {
    /// The state before each entry.
    befores: Vec<(f32, f32, u8)>,
    /// The plain value at its first write since the history last ticked.
    value_before: Option<u8>,
    scope_changed: bool,
    scope_before: (f32, f32, u8),
}

impl Model {
    /// What a tick of the history does to the plain value's writes: one
    /// step if they changed it, joining the open scope or its own entry.
    fn flush(&mut self, now: (f32, f32, u8), in_scope: bool) {
        if let Some(before) = self.value_before.take()
            && before != now.2
        {
            if in_scope {
                self.scope_changed = true;
            } else {
                self.befores.push((now.0, now.1, before));
            }
        }
    }
}
