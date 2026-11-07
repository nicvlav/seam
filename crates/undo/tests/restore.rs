//! Restore neutrality: a restore clears the history and is never recorded as
//! an edit.

use std::sync::{Arc, Barrier};
use std::time::Duration;

use seam_params::{
    Kind, List, Origin, ParamDecl, ParamId, ParamSet, ParamSetBuilder, RestoreReader,
    RestoreWriter, UiParams, Value,
};
use seam_testkit::sync::wait_until;
use seam_undo::{Bound, History};

struct Rig {
    set: Arc<ParamSet>,
    ui: UiParams,
    clock: RestoreReader,
    writer: RestoreWriter,
    history: History,
    p: ParamId,
}

fn rig() -> Rig {
    let mut builder = ParamSetBuilder::new();
    builder
        .add(ParamDecl::new(
            "p",
            "p",
            Kind::continuous(0.0, 10.0, 1.0, None),
            5.0,
        ))
        .expect("valid");
    let set = builder.build();
    let clock = std::sync::Arc::clone(set.clock());
    let ui = set.take_ui().expect("the first UI handle");
    let history = History::new(&ui, Bound::default()).expect("the first history");
    history.enroll_params();
    let p = set.id("p").expect("declared");
    Rig {
        set,
        ui,
        clock: clock.reader(),
        writer: clock.writer(),
        history,
        p,
    }
}

impl Rig {
    /// A whole restore of `p` to `value`, as the session runs it: stored,
    /// then applied under the mark, then completed.
    fn restore(&self, value: f32) {
        let load = self.set.program().load();
        {
            let _applying = self.writer.applying();
            load.restore(self.p, value);
        }
        self.writer.completed();
    }
}

#[test]
fn history_from_before_a_restore_is_cleared_and_never_replays() {
    // Clearing the history keeps an old list step from replaying by index
    // over the restored list.
    let rig = rig();
    let value = Value::new(1_i32, &rig.set);
    let list = List::new(vec![1_u8, 2], &rig.set);
    rig.history.enroll(&value, "v").expect("first");
    rig.history.enroll_list(&list, "list").expect("first");
    assert!(rig.ui.write(rig.p, 8.0));
    value.set(2);
    assert!(list.remove(0));
    rig.history.tick();
    assert_eq!(rig.history.len(), 3);

    rig.restore(3.0);
    list.reset(vec![9]);
    assert!(
        rig.history.entries().is_empty(),
        "the listing reads the clock"
    );
    assert_eq!(rig.history.undo(), Ok(false));
    assert_eq!((rig.set.value(rig.p), value.get(), list.len()), (3.0, 2, 1));
}

#[test]
fn an_undo_between_a_loads_writes_and_its_store_replays_nothing() {
    // A load marks the restore stored before its first write, so an undo
    // pressed before the session applies finds the history cleared.
    let rig = rig();
    assert!(rig.ui.write(rig.p, 8.0));
    rig.history.tick();
    let load = rig.set.program().load();
    load.restore(rig.p, 2.0);
    assert_eq!(rig.history.undo(), Ok(false));
    assert_eq!(rig.set.value(rig.p), 2.0);
}

#[test]
fn a_step_recorded_after_a_load_began_never_replays_over_its_write() {
    // The user's write lands after the load marked its restore stored, a
    // tick records it, and only then does the load reach the parameter.
    // That write marks the restore stored again, so the undo replays
    // nothing.
    let rig = rig();
    let load = rig.set.program().load();
    assert!(rig.ui.write(rig.p, 8.0));
    rig.history.tick();
    assert_eq!(rig.history.len(), 1, "recorded after the load began");
    load.restore(rig.p, 2.0);
    assert_eq!(rig.history.undo(), Ok(false));
    assert_eq!(rig.set.value(rig.p), 2.0, "the loaded value stands");
}

#[test]
fn steps_recorded_between_a_store_and_its_apply_are_dropped_at_completion() {
    // The pending restore outranks live edits.
    let rig = rig();
    let value = Value::new(1_i32, &rig.set);
    rig.history.enroll(&value, "v").expect("first");
    rig.writer.stored();
    rig.history.tick();
    assert!(rig.ui.write(rig.p, 8.0));
    value.set(2);
    rig.history.tick();
    assert_eq!(rig.history.len(), 2, "recorded inside the window");

    {
        let _applying = rig.writer.applying();
        value.set(7);
    }
    rig.writer.completed();
    assert!(rig.history.is_empty(), "dropped once the apply completed");
    value.set(8);
    rig.history.tick();
    rig.history.undo().expect("idle");
    assert_eq!(value.get(), 7, "the baseline followed the restored value");
}

#[test]
fn a_record_or_commit_from_before_a_restore_never_records_after_it() {
    // A gesture and a plain value change made before a restore, not yet
    // ticked when it lands.
    let rig = rig();
    let value = Value::new(1_i32, &rig.set);
    let list = List::new(vec![1_u8, 2], &rig.set);
    rig.history.enroll(&value, "v").expect("first");
    rig.history.enroll_list(&list, "list").expect("first");
    assert!(rig.ui.write(rig.p, 8.0));
    value.set(2);
    // A list this restore leaves alone: its verb from before still records
    // nothing.
    assert!(list.remove(0));
    rig.restore(3.0);
    rig.history.tick();
    assert!(rig.history.is_empty());

    value.set(4);
    rig.history.tick();
    rig.history.undo().expect("idle");
    assert_eq!(
        value.get(),
        2,
        "the baseline still followed the stale change"
    );
}

#[test]
fn writes_under_the_applying_mark_record_nothing_and_clear_nothing() {
    // A late participant's apply records nothing and clears nothing.
    let rig = rig();
    let value = Value::new(1_i32, &rig.set);
    let late = Value::new(String::from("a"), &rig.set);
    rig.history.enroll(&value, "v").expect("first");
    rig.history.enroll(&late, "late").expect("first");
    value.set(2);
    rig.history.tick();
    {
        let _applying = rig.writer.applying();
        late.set("restored".into());
    }
    rig.history.tick();
    assert_eq!(rig.history.len(), 1, "nothing recorded, nothing cleared");
    late.set("b".into());
    rig.history.tick();
    rig.history.undo().expect("idle");
    assert_eq!(late.get(), "restored");
}

#[test]
fn a_plain_value_gesture_open_across_a_restore_records_from_the_restored_value() {
    let rig = rig();
    let value = Value::new(1_i32, &rig.set);
    rig.history.enroll(&value, "v").expect("first");
    let drag = value.begin();
    drag.set(2);
    rig.writer.stored();
    {
        let _applying = rig.writer.applying();
        value.set(5);
    }
    rig.writer.completed();
    rig.history.tick();
    drag.set(6);
    drop(drag);
    rig.history.tick();
    assert_eq!(rig.history.len(), 1);
    rig.history.undo().expect("idle");
    assert_eq!(value.get(), 5);
}

#[test]
fn a_parameter_drag_across_a_restore_keeps_its_before_and_the_restore_stands() {
    // The drag's step runs from the pre-drag value to the drag's
    // last value, but its close writes nothing, so the restored value
    // stands; undoing the step writes the pre-drag value.
    let rig = rig();
    let drag = rig.ui.begin(rig.p);
    assert!(drag.write(6.0));
    rig.restore(2.0);
    rig.history.tick();
    drop(drag);
    assert_eq!(rig.set.value(rig.p), 2.0);
    rig.history.tick();
    assert_eq!(rig.history.len(), 1);
    rig.history.undo().expect("idle");
    assert_eq!(rig.set.value(rig.p), 5.0);
    rig.history.redo().expect("idle");
    assert_eq!(rig.set.value(rig.p), 6.0);
}

#[test]
fn a_plain_value_gesture_across_a_restore_moves_the_baseline_with_no_write_after() {
    // The drag writes nothing after the restore, so the next
    // change starts from the restored value, for a full restore and for a
    // late participant's apply alike.
    for full in [true, false] {
        let rig = rig();
        let value = Value::new(0_i32, &rig.set);
        rig.history.enroll(&value, "v").expect("first");
        let drag = value.begin();
        drag.set(5);
        if full {
            rig.writer.stored();
        }
        {
            let _applying = rig.writer.applying();
            value.set(9);
        }
        if full {
            rig.writer.completed();
        }
        drop(drag);
        rig.history.tick();
        assert!(rig.history.is_empty(), "full: {full}");
        value.set(7);
        rig.history.tick();
        rig.history.undo().expect("idle");
        assert_eq!(value.get(), 9, "full: {full}");
    }
}

#[test]
fn list_verbs_made_after_a_reset_an_apply_or_a_restore_undo_as_usual() {
    // A reset or an apply makes only earlier steps stale.
    let rig = rig();
    let reset = List::new(vec![1_u8, 2], &rig.set);
    let applied = List::new(vec![1_u8, 2], &rig.set);
    let restored = List::new(vec![1_u8, 2], &rig.set);
    rig.history.enroll_list(&reset, "reset").expect("first");
    rig.history.enroll_list(&applied, "applied").expect("first");
    rig.history
        .enroll_list(&restored, "restored")
        .expect("first");

    reset.reset(vec![7, 8]);
    {
        let _applying = rig.writer.applying();
        applied.push(3);
    }
    rig.restore(3.0);
    reset.push(9);
    applied.push(4);
    restored.push(5);
    rig.history.tick();
    assert_eq!(rig.history.len(), 3, "each verb made after recorded");
    rig.history.jump(0).expect("idle");
    assert_eq!(reset.with(<[u8]>::to_vec), [7, 8]);
    assert_eq!(applied.with(<[u8]>::to_vec), [1, 2, 3]);
    assert_eq!(restored.with(<[u8]>::to_vec), [1, 2]);
    rig.history.jump(3).expect("idle");
    assert_eq!(reset.with(<[u8]>::to_vec), [7, 8, 9]);
}

#[test]
fn plain_value_writes_coalesce_only_within_one_stamp() {
    // A write made after a restore completed is not swallowed by one
    // made before it.
    let rig = rig();
    let value = Value::new(0_i32, &rig.set);
    rig.history.enroll(&value, "v").expect("first");
    value.set(1);
    rig.writer.stored();
    rig.writer.completed();
    value.set(2);
    rig.history.tick();
    assert_eq!(rig.history.len(), 1);
    rig.history.undo().expect("idle");
    assert_eq!(
        value.get(),
        1,
        "the baseline followed the write from before"
    );
}

#[test]
fn a_write_under_the_applying_mark_is_the_restores_and_nothing_derived_records() {
    // A value derived from an applied one passes Restore on.
    let rig = rig();
    let source = Value::new(1_i32, &rig.set);
    let derived = Value::new(10_i32, &rig.set);
    rig.history.enroll(&derived, "derived").expect("first");
    {
        let _applying = rig.writer.applying();
        source.set_from(2, Origin::User);
    }
    let reading = source.read();
    assert_eq!(reading.origin, Origin::Restore);
    derived.set_from(reading.value * 10, reading.origin);
    rig.history.tick();
    assert!(rig.history.is_empty());
}

#[test]
fn the_listing_counter_reads_the_clock() {
    // A panel polling only the counter redraws after a restore.
    let rig = rig();
    assert!(rig.ui.write(rig.p, 8.0));
    rig.history.tick();
    let version = rig.history.version();
    rig.restore(3.0);
    assert!(rig.history.version() > version);
    assert!(rig.history.is_empty());
}

#[test]
fn a_transaction_open_across_a_clear_or_a_restore_starts_a_new_entry() {
    // The cleared entry is gone, so later steps cannot join it.
    for restore in [false, true] {
        let rig = rig();
        let value = Value::new(0_i32, &rig.set);
        rig.history.enroll(&value, "v").expect("first");
        let transaction = rig.history.transaction("t");
        value.set(1);
        rig.history.tick();
        assert_eq!(rig.history.len(), 1);
        if restore {
            rig.restore(3.0);
        } else {
            rig.history.clear().expect("idle");
        }
        value.set(2);
        rig.history.tick();
        assert_eq!(rig.history.len(), 1, "restore: {restore}");
        value.set(3);
        rig.history.tick();
        assert_eq!(rig.history.len(), 1, "joined the new entry");
        drop(transaction);
        rig.history.undo().expect("idle");
        assert_eq!(value.get(), 1, "restore: {restore}");
    }
}

#[test]
fn a_load_waits_while_a_replay_holds_loads_off() {
    // The hold a replay step takes keeps a load's write out until the
    // step has written, and the write's mark of the restore with it, so
    // the step's check under the hold never misses a write that landed.
    let rig = rig();
    let held = rig.clock.hold_loads();
    let set = Arc::clone(&rig.set);
    let p = rig.p;
    let loader = std::thread::spawn(move || set.program().load().restore(p, 2.0));
    wait_until("the load to store its restore", || {
        rig.clock.stamp().stored() > 0
    });
    // A bounded negative check: a slow machine can only make it pass
    // wrongly, never fail wrongly. The deterministic pin of the replay
    // side is the unit test in crates/undo/src/history/tests.rs.
    std::thread::sleep(Duration::from_millis(20));
    assert_eq!(rig.set.value(rig.p), 5.0, "the load waits");
    assert_eq!(
        rig.clock.stamp().stored(),
        1,
        "only the load's start marked"
    );
    drop(held);
    loader.join().expect("the loading thread");
    assert_eq!(rig.set.value(rig.p), 2.0);
    assert_eq!(rig.clock.stamp().stored(), 2, "and then its write");
}

#[test]
fn a_load_racing_an_undo_always_ends_on_the_loaded_value() {
    // An undo never overwrites a loaded value, whichever wins.
    for _ in 0..200 {
        let rig = rig();
        assert!(rig.ui.write(rig.p, 8.0));
        rig.history.tick();
        let set = Arc::clone(&rig.set);
        let p = rig.p;
        let start = Arc::new(Barrier::new(2));
        let go = Arc::clone(&start);
        let loader = std::thread::spawn(move || {
            go.wait();
            set.program().load().restore(p, 2.0);
        });
        start.wait();
        let _ = rig.history.undo();
        loader.join().expect("the loading thread");
        assert_eq!(rig.set.value(rig.p), 2.0);
    }
}
