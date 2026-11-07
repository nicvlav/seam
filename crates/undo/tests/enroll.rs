//! Enrollment and recording: which writes become steps, and what each step
//! holds.

use std::cell::RefCell;
use std::sync::Arc;

use proptest::prelude::*;
use proptest::test_runner::FileFailurePersistence;
use seam_params::{
    Kind, List, Origin, ParamDecl, ParamId, ParamSet, ParamSetBuilder, RestoreWriter, UiParams,
    Value,
};
use seam_undo::{Bound, History, HistoryError};

fn param(key: &'static str) -> ParamDecl {
    ParamDecl::new(key, key, Kind::continuous(0.0, 10.0, 1.0, None), 5.0)
}

struct Rig {
    set: Arc<ParamSet>,
    ui: UiParams,
    writer: RestoreWriter,
    history: History,
    ids: Vec<ParamId>,
}

fn rig(keys: &[&'static str]) -> Rig {
    let mut builder = ParamSetBuilder::new();
    for &key in keys {
        builder.add(param(key)).expect("valid");
    }
    let set = builder.build();
    let clock = std::sync::Arc::clone(set.clock());
    let ui = set.take_ui().expect("the first UI handle");
    let history = History::new(&ui, Bound::default()).expect("the first history");
    let ids = set.ids().collect();
    Rig {
        set,
        ui,
        writer: clock.writer(),
        history,
        ids,
    }
}

impl Rig {
    fn value<T: Clone>(&self, initial: T) -> Value<T> {
        Value::new(initial, &self.set)
    }

    fn names(&self) -> Vec<String> {
        self.history
            .entries()
            .into_iter()
            .map(|entry| entry.name)
            .collect()
    }
}

#[test]
fn a_value_is_enrolled_once_in_every_build() {
    let rig = rig(&["p"]);
    let p = rig.ids[0];
    assert_eq!(rig.history.enroll_param(p), Ok(()));
    assert_eq!(
        rig.history.enroll_param(p),
        Err(HistoryError::AlreadyEnrolled)
    );

    let value = rig.value(1_i32);
    assert_eq!(rig.history.enroll(&value, "v"), Ok(()));
    assert_eq!(
        rig.history.enroll(&value, "v"),
        Err(HistoryError::AlreadyEnrolled)
    );
    // One history per UI handle; a history over another plugin instance's
    // parameters refuses the value too, since that instance's set made
    // another clock.
    assert_eq!(
        History::new(&rig.ui, Bound::default()).err(),
        Some(HistoryError::HistoryExists)
    );
    let other_set = ParamSetBuilder::new().build();
    let other_ui = other_set.take_ui().expect("its UI handle");
    let other = History::new(&other_ui, Bound::default()).expect("its history");
    assert_eq!(other.enroll(&value, "v"), Err(HistoryError::OtherClock));

    value.set(2);
    rig.history.tick();
    assert_eq!(rig.history.len(), 1, "one step, not two");
}

#[test]
fn enrolling_records_nothing_and_earlier_writes_never_record() {
    let rig = rig(&["p"]);
    let p = rig.ids[0];
    assert!(rig.ui.write(p, 7.0));
    let value = rig.value(1_i32);
    value.set(2);
    rig.history.enroll_param(p).expect("first");
    rig.history.enroll(&value, "v").expect("first");
    rig.history.tick();
    assert!(rig.history.is_empty());
    assert_eq!(rig.history.undo(), Ok(false));
    assert_eq!((rig.set.value(p), value.get()), (7.0, 2));
}

#[test]
fn a_gesture_open_at_enrollment_records_from_its_open_value() {
    let rig = rig(&["p"]);
    let p = rig.ids[0];

    let value = rig.value(1_i32);
    let drag = value.begin();
    drag.set(2);
    rig.history.enroll(&value, "Value").expect("first");
    drag.set(3);
    drag.set(4);
    rig.history.tick();
    assert!(
        rig.history.is_empty(),
        "writes inside the gesture are tracked, not recorded"
    );
    drop(drag);
    assert_eq!(rig.history.undo(), Ok(true));
    assert_eq!(value.get(), 1);
    assert_eq!(rig.history.undo(), Ok(false));
    assert_eq!(rig.history.redo(), Ok(true));
    assert_eq!(value.get(), 4);

    let drag = rig.ui.begin(p);
    assert!(drag.write(4.0));
    rig.history.enroll_param(p).expect("first");
    assert!(drag.write(7.0));
    drop(drag);
    rig.history.tick();
    assert_eq!(rig.history.undo(), Ok(true));
    assert_eq!(
        rig.set.value(p),
        5.0,
        "the pre-drag value, not the one at enrollment"
    );

    // The same for a plain value: it records 5 -> 6.
    let rig = self::rig(&["p"]);
    let value = rig.value(5_i32);
    let drag = value.begin();
    drag.set(6);
    rig.history.enroll(&value, "Value").expect("first");
    drop(drag);
    rig.history.tick();
    assert_eq!(rig.history.undo(), Ok(true));
    assert_eq!(value.get(), 5);
}

#[test]
fn a_gesture_is_one_step_and_a_host_write_during_it_never_joins() {
    // A host write during a gesture cannot taint the recorded step.
    let rig = rig(&["p"]);
    let p = rig.ids[0];
    rig.history.enroll_param(p).expect("first");
    let audio = rig.set.audio();

    let drag = rig.ui.begin(p);
    for value in [6.0, 7.0, 8.0] {
        assert!(drag.write(value));
        audio.host_write(p, 0.2);
    }
    drop(drag);
    assert_eq!(
        rig.set.value(p),
        2.0,
        "the close writes nothing: the host's value stands"
    );
    rig.history.tick();
    assert_eq!(rig.names(), ["p"]);
    assert_eq!(rig.history.undo(), Ok(true));
    assert_eq!(rig.set.value(p), 5.0);
    assert_eq!(rig.history.redo(), Ok(true));
    assert_eq!(
        rig.set.value(p),
        8.0,
        "the after-value is the user's last write"
    );

    // A drag back to where it started is no step.
    let drag = rig.ui.begin(p);
    assert!(drag.write(3.0));
    assert!(drag.write(8.0));
    drop(drag);
    rig.history.tick();
    assert_eq!(rig.history.len(), 1);
}

#[test]
fn outside_a_gesture_a_value_records_each_change_and_a_parameter_nothing() {
    let rig = rig(&["p"]);
    let p = rig.ids[0];
    rig.history.enroll_param(p).expect("first");
    let value = rig.value(String::from("a"));
    rig.history.enroll(&value, "Sample").expect("first");

    rig.set.audio().host_write(p, 0.9);
    let mut edit = rig.ui.edit();
    assert!(edit.write(p, 3.0));
    drop(edit);
    assert!(rig.set.program().sync(p, 4.0));
    rig.history.tick();
    assert!(
        rig.history.is_empty(),
        "no host, edit or sync write records"
    );

    value.set("b".into());
    rig.history.tick();
    value.set("c".into());
    rig.history.tick();
    assert_eq!(rig.names(), ["Sample", "Sample"]);
    rig.history.undo().expect("idle");
    assert_eq!(value.get(), "b");
    rig.history.undo().expect("idle");
    assert_eq!(value.get(), "a");
}

#[test]
fn writes_between_two_ticks_coalesce_latest_winning() {
    let rig = rig(&[]);
    let value = rig.value(0_u32);
    rig.history.enroll(&value, "v").expect("first");
    let writer = value.clone();
    std::thread::spawn(move || {
        for n in 1..=1000 {
            writer.set(n);
        }
    })
    .join()
    .expect("the writer");
    rig.history.tick();
    assert_eq!(rig.history.len(), 1);
    rig.history.undo().expect("idle");
    assert_eq!(value.get(), 0);
    rig.history.redo().expect("idle");
    assert_eq!(value.get(), 1000);
}

#[test]
fn a_no_op_write_keeps_the_redo_nan_payloads_included() {
    let rig = rig(&[]);
    let payload = f32::from_bits(0x7FC0_1234);
    let value = rig.value(1.0_f32);
    rig.history.enroll(&value, "v").expect("first");
    value.set(payload);
    rig.history.tick();
    value.set(2.0);
    rig.history.tick();
    rig.history.undo().expect("idle");
    assert_eq!(value.get().to_bits(), payload.to_bits());

    value.set(payload);
    rig.history.tick();
    value.set(-0.0);
    value.set(payload);
    rig.history.tick();
    assert_eq!(rig.history.redo(), Ok(true), "the redo survived");
    assert_eq!(value.get(), 2.0);

    let zero = rig.value(0.0_f32);
    rig.history.enroll(&zero, "zero").expect("first");
    zero.set(-0.0);
    rig.history.tick();
    assert_eq!(rig.history.len(), 2, "a sign flip of zero is no change");
}

#[test]
fn the_next_change_after_a_replay_starts_from_what_the_replay_left() {
    let rig = rig(&[]);
    let value = rig.value(1_i32);
    rig.history.enroll(&value, "v").expect("first");
    value.set(2);
    rig.history.tick();
    rig.history.undo().expect("idle");
    rig.history.tick();
    assert_eq!(rig.history.len(), 1, "the replay recorded nothing");
    value.set(3);
    rig.history.tick();
    assert_eq!(rig.history.len(), 1, "the new step dropped the redo");
    rig.history.undo().expect("idle");
    assert_eq!(value.get(), 1, "not 2");
}

#[test]
fn a_write_derived_from_a_change_records_nothing_and_follows_its_replays() {
    // A write cascaded from a deferred replay drain never records: its
    // source's step covers it, and it must not wipe the redo.
    let rig = rig(&[]);
    let source = rig.value(1_i32);
    let derived = rig.value(10_i32);
    rig.history.enroll(&source, "source").expect("first");
    rig.history.enroll(&derived, "derived").expect("first");
    let derive = || {
        let reading = source.read();
        derived.set_from(reading.value * 10, reading.origin);
    };

    source.set(2);
    derive();
    rig.history.tick();
    assert_eq!(rig.history.len(), 1, "one user action, one step");
    rig.history.jump(0).expect("idle");
    assert_eq!(source.read().origin, Origin::Replay);
    derive();
    rig.history.tick();
    assert_eq!(rig.history.len(), 1);
    assert_eq!(derived.get(), 10);
    assert_eq!(rig.history.redo(), Ok(true), "the redo survived");
    derive();
    assert_eq!(derived.get(), 20);
}

/// A payload whose instances share one count, so a test can tell when a
/// value's own copy is freed.
#[derive(Debug, Clone)]
struct Counted {
    _count: Arc<()>,
    n: u8,
}

impl Counted {
    fn new(count: &Arc<()>, n: u8) -> Self {
        Counted {
            _count: Arc::clone(count),
            n,
        }
    }
}

impl seam_undo::UndoSize for Counted {}

impl seam_undo::UndoEq for Counted {
    fn undo_eq(&self, other: &Self) -> bool {
        self.n == other.n
    }
}

#[test]
fn forgetting_one_value_leaves_another_of_its_type_enrolled() {
    // A value is forgotten by what it is, not by its type.
    let rig = rig(&[]);
    let (a, b, never) = (rig.value(1_i32), rig.value(1_i32), rig.value(1_i32));
    rig.history.enroll(&a, "a").expect("first");
    rig.history.enroll(&b, "b").expect("first");
    assert!(rig.history.forget(&b));
    assert!(!rig.history.forget(&never), "never enrolled");
    a.set(2);
    b.set(2);
    rig.history.tick();
    assert_eq!(rig.history.len(), 1, "a still records, b no more");
    assert!(rig.history.forget(&a));
}

#[test]
fn a_forgotten_or_dropped_value_replays_as_nothing() {
    // Steps outlive enrollments, values drop in any order, and the history
    // never keeps a value alive.
    let rig = rig(&["p"]);
    let p = rig.ids[0];
    rig.history.enroll_param(p).expect("first");
    let kept = rig.value(1_i32);
    let count = Arc::new(());
    let dropped = rig.value(Counted::new(&count, 0));
    rig.history.enroll(&kept, "kept").expect("first");
    rig.history.enroll(&dropped, "dropped").expect("first");
    assert!(rig.ui.write(p, 9.0));
    kept.set(2);
    dropped.set(Counted::new(&count, 1));
    rig.history.tick();
    assert_eq!(rig.history.len(), 3);

    rig.history.forget_param(p);
    assert!(rig.history.forget(&kept));
    assert!(!rig.history.forget(&kept));
    let held = Arc::strong_count(&count);
    drop(dropped);
    assert_eq!(
        Arc::strong_count(&count),
        held - 1,
        "the value's own copy freed with its last handle"
    );
    rig.history.tick();
    assert_eq!(
        Arc::strong_count(&count),
        held - 2,
        "the tick forgot the dead enrollment and its baseline"
    );
    rig.history.jump(0).expect("idle");
    assert_eq!((rig.set.value(p), kept.get()), (9.0, 2));
    assert_eq!(rig.history.len(), 3, "the steps stay listed");
    rig.history.jump(3).expect("idle");
    assert_eq!(
        Arc::strong_count(&count),
        held - 2,
        "no replay brought the value back"
    );

    // Enrolled again, the value records anew; the old steps stay inert.
    rig.history.enroll(&kept, "kept").expect("again");
    rig.history.redo().expect("idle");
    rig.history.redo().expect("idle");
    assert_eq!(kept.get(), 2);
}

#[test]
fn undo_waits_while_a_plain_value_gesture_is_open() {
    let rig = rig(&[]);
    let value = rig.value(1_i32);
    rig.history.enroll(&value, "v").expect("first");
    value.set(2);
    let drag = value.begin();
    assert_eq!(rig.history.undo(), Err(HistoryError::GestureOpen));
    drag.set(3);
    drop(drag);
    assert_eq!(rig.history.undo(), Ok(true));
    assert_eq!(value.get(), 2);
}

#[test]
fn only_a_plain_values_own_writes_record() {
    // A value mirroring automation records nothing, nor does any write
    // through set_from; set records.
    let rig = rig(&[]);
    let value = rig.value(0_i32);
    rig.history.enroll(&value, "v").expect("first");
    value.set_from(3, Origin::Host);
    value.set_from(4, Origin::Learn);
    value.set_from(5, Origin::Program);
    value.set_from(6, Origin::User);
    rig.history.tick();
    assert!(rig.history.is_empty());
    value.set(7);
    rig.history.tick();
    assert_eq!(rig.history.len(), 1);
    rig.history.undo().expect("idle");
    assert_eq!(value.get(), 6, "the baseline followed every derived write");
}

#[test]
fn a_change_derived_from_a_parameter_or_a_list_records_nothing() {
    // One rule for every kind, and one user action is one step however
    // many values follow it.
    let rig = rig(&["p"]);
    let p = rig.ids[0];
    rig.history.enroll_param(p).expect("first");
    let list = List::new(vec![1_u8, 2], &rig.set);
    rig.history.enroll_list(&list, "list").expect("first");
    let shown = rig.value(0.0_f32);
    let count = rig.value(2_usize);
    rig.history.enroll(&shown, "shown").expect("first");
    rig.history.enroll(&count, "count").expect("first");
    let derive = || {
        let reading = rig.set.read(p);
        shown.set_from(reading.value * 2.0, reading.origin);
        let reading = list.read(<[_]>::len);
        count.set_from(reading.value, reading.origin);
    };

    // A drag over several ticks, its derived value following each one.
    let drag = rig.ui.begin(p);
    for value in [6.0, 7.0, 8.0] {
        assert!(drag.write(value));
        derive();
        rig.history.tick();
    }
    drop(drag);
    rig.history.tick();
    assert!(list.remove(0));
    derive();
    rig.history.tick();
    assert_eq!(rig.history.len(), 2, "the drag and the remove");

    rig.history.jump(0).expect("idle");
    assert_eq!(rig.set.read(p).origin, Origin::Replay);
    assert_eq!(list.read(<[_]>::len).origin, Origin::Replay);
    derive();
    rig.history.tick();
    assert_eq!((shown.get(), count.get()), (10.0, 2));
    assert_eq!(rig.history.len(), 2);

    // A sync records nothing for the parameter, nor through what follows
    // it, so the redo survives.
    assert!(rig.set.program().sync(p, 3.0));
    derive();
    rig.history.tick();
    assert_eq!(rig.history.redo(), Ok(true), "the redo survived");
}

#[test]
fn a_list_kept_in_step_with_a_value_records_nothing_and_follows_its_replays() {
    // Derived verbs record nothing, and each makes the list's earlier steps
    // stale, since it changed what they index.
    let rig = rig(&[]);
    let count = rig.value(0_usize);
    let list = List::new(Vec::<u8>::new(), &rig.set);
    rig.history.enroll(&count, "count").expect("first");
    rig.history.enroll_list(&list, "list").expect("first");
    let follow = || {
        let reading = count.read();
        while list.len() < reading.value {
            list.derived(reading.origin).push(0);
        }
        while list.len() > reading.value {
            list.derived(reading.origin).remove(list.len() - 1);
        }
    };

    list.push(9);
    rig.history.tick();
    count.set(2);
    follow();
    rig.history.tick();
    assert_eq!(
        rig.history.len(),
        2,
        "the push and the count, not what followed"
    );
    rig.history.undo().expect("idle");
    assert_eq!(count.read().origin, Origin::Replay);
    follow();
    assert_eq!(list.len(), 0);
    rig.history.tick();
    assert_eq!(rig.history.redo(), Ok(true), "the redo survived");
    follow();
    assert_eq!(list.len(), 2);
    rig.history.jump(0).expect("idle");
    follow();
    assert_eq!(
        list.len(),
        0,
        "the push's own step went stale, so it did not remove a derived element"
    );
}

#[test]
fn one_tick_records_steps_in_the_order_they_happened() {
    // One undo between dependent changes lands on a state that existed. The
    // selection was enrolled first, yet changed last.
    let rig = rig(&["p"]);
    let p = rig.ids[0];
    let selection = rig.value(1_usize);
    let list = List::new(vec![1_u8, 2], &rig.set);
    rig.history.enroll(&selection, "selection").expect("first");
    rig.history.enroll_list(&list, "list").expect("first");
    rig.history.enroll_param(p).expect("first");

    selection.set(0);
    list.push(3);
    assert!(rig.ui.write(p, 8.0));
    selection.set(2);
    rig.history.tick();
    assert_eq!(rig.names(), ["list", "p", "selection"]);
    rig.history.undo().expect("idle");
    assert_eq!((list.len(), selection.get()), (3, 1));
    rig.history.undo().expect("idle");
    assert_eq!(rig.set.value(p), 5.0);
    rig.history.undo().expect("idle");
    assert_eq!(list.len(), 2);
}

thread_local! {
    static HOOK: std::cell::RefCell<Option<Box<dyn FnOnce()>>> = const { std::cell::RefCell::new(None) };
}

/// Runs `f` at the next comparison of a `Hooked`, which the tick makes
/// while it polls: another thread's write landing mid-tick, made
/// deterministic.
fn during_the_tick(f: impl FnOnce() + 'static) {
    HOOK.with(|hook| *hook.borrow_mut() = Some(Box::new(f)));
}

#[derive(Debug, Clone)]
struct Hooked(u8);

impl seam_undo::UndoSize for Hooked {}

impl seam_undo::UndoEq for Hooked {
    fn undo_eq(&self, other: &Self) -> bool {
        if let Some(f) = HOOK.with(|hook| hook.borrow_mut().take()) {
            f();
        }
        self.0 == other.0
    }
}

#[test]
fn changes_made_during_a_tick_wait_for_the_next_one_in_order() {
    // A worker pushes an element and selects it while the tick runs,
    // after it took the list's verbs and before the selection's. Both wait
    // for the next tick, so one undo leaves the selection in the list.
    let rig = rig(&[]);
    let list = List::new(vec![1_u8, 2], &rig.set);
    let probe = rig.value(Hooked(0));
    let selection = rig.value(1_usize);
    rig.history.enroll_list(&list, "list").expect("first");
    rig.history.enroll(&probe, "probe").expect("first");
    rig.history.enroll(&selection, "selection").expect("first");
    probe.set(Hooked(1));
    let (worker_list, worker_selection) = (list.clone(), selection.clone());
    during_the_tick(move || {
        worker_list.push(3);
        worker_selection.set(2);
    });
    rig.history.tick();
    assert_eq!(rig.names(), ["probe"], "both waited");
    rig.history.tick();
    assert_eq!(rig.names(), ["probe", "list", "selection"]);
    rig.history.undo().expect("idle");
    assert_eq!((selection.get(), list.len()), (1, 3));

    // A list polled after the worker's write waits too, so the selection
    // the worker set first still records first.
    let rig = self::rig(&[]);
    let list = List::new(vec![1_u8, 2], &rig.set);
    let probe = rig.value(Hooked(0));
    let selection = rig.value(1_usize);
    rig.history.enroll(&probe, "probe").expect("first");
    rig.history.enroll_list(&list, "list").expect("first");
    rig.history.enroll(&selection, "selection").expect("first");
    probe.set(Hooked(1));
    let (worker_list, worker_selection) = (list.clone(), selection.clone());
    during_the_tick(move || {
        worker_selection.set(0);
        worker_list.push(3);
    });
    rig.history.tick();
    rig.history.tick();
    assert_eq!(rig.names(), ["probe", "selection", "list"]);
}

#[test]
fn a_derived_write_inside_a_drag_follows_its_origin() {
    // A linked value echoing the user's drag back keeps the drag; a value
    // mirroring automation is never the drag's before or after.
    let rig = rig(&[]);
    let value = rig.value(0_i32);
    rig.history.enroll(&value, "v").expect("first");
    let drag = value.begin();
    drag.set(2);
    value.set_from(3, Origin::User);
    drop(drag);
    rig.history.tick();
    assert_eq!(rig.history.len(), 1, "one step for the drag");
    rig.history.undo().expect("idle");
    assert_eq!(value.get(), 0);
    rig.history.redo().expect("idle");
    assert_eq!(
        value.get(),
        3,
        "the echo joined the drag as its after-value"
    );

    let drag = value.begin();
    drag.set(4);
    value.set_from(7, Origin::Host);
    drag.set(5);
    drop(drag);
    rig.history.tick();
    rig.history.undo().expect("idle");
    assert_eq!(value.get(), 3, "the value at the open, not the host's");
    rig.history.redo().expect("idle");
    assert_eq!(value.get(), 5, "the user's last write");
}

#[test]
fn a_value_or_list_made_with_another_clock_is_refused() {
    // Its numbers would never fall before the history's cut. Only another
    // set makes another clock.
    let rig = rig(&[]);
    let other = ParamSetBuilder::new().build();
    let value = Value::new(0_i32, &other);
    assert_eq!(
        rig.history.enroll(&value, "v"),
        Err(HistoryError::OtherClock)
    );
    let list = List::new(vec![1_u8], &other);
    assert_eq!(
        rig.history.enroll_list(&list, "list"),
        Err(HistoryError::OtherClock)
    );
    assert_eq!(rig.history.enroll(&rig.value(0_i32), "ours"), Ok(()));
}

#[test]
fn only_a_parameters_enrolment_ticks_and_one_enrolled_already_never_does() {
    // A pending step waits through a value's or a list's enrolment, taken or
    // refused, and through the enrolment of a parameter enrolled already.
    let rig = rig(&["p", "q"]);
    let (p, q) = (rig.ids[0], rig.ids[1]);
    rig.history.enroll_param(p).expect("first");
    let value = rig.value(0_i32);
    rig.history.enroll(&value, "v").expect("first");
    value.set(1);

    let other = ParamSetBuilder::new().build();
    assert_eq!(
        rig.history.enroll(&Value::new(0_i32, &other), "other"),
        Err(HistoryError::OtherClock)
    );
    assert_eq!(
        rig.history
            .enroll_list(&List::new(vec![1_u8], &other), "other"),
        Err(HistoryError::OtherClock)
    );
    assert_eq!(
        rig.history.enroll(&value, "v"),
        Err(HistoryError::AlreadyEnrolled)
    );
    assert_eq!(
        rig.history.enroll_param(p),
        Err(HistoryError::AlreadyEnrolled)
    );
    rig.history.enroll(&rig.value(0_u8), "u").expect("first");
    rig.history
        .enroll_list(&List::new(vec![1_u8], &rig.set), "list")
        .expect("first");
    assert!(rig.history.is_empty(), "nothing recorded yet");

    // Taking the parameter's earlier gestures, its enrolment records the
    // pending step too.
    rig.history.enroll_param(q).expect("first");
    assert_eq!(rig.names(), ["v"]);
}

thread_local! {
    /// What the next drop of an `Enrols` enrols, once.
    static ENROL: RefCell<Option<(History, ParamId)>> = const { RefCell::new(None) };
}

/// A plain value whose drop enrols the parameter `ENROL` holds: a
/// destructor a tick runs may run any code.
#[derive(Clone)]
struct Enrols;

impl seam_undo::UndoSize for Enrols {}

impl seam_undo::UndoEq for Enrols {
    fn undo_eq(&self, _: &Self) -> bool {
        true
    }
}

impl Drop for Enrols {
    fn drop(&mut self) {
        if let Some((history, id)) = ENROL.with(|armed| armed.borrow_mut().take()) {
            assert_eq!(history.enroll_param(id), Ok(()));
        }
    }
}

#[test]
fn an_enrolment_a_destructor_beats_inside_its_tick_is_refused_after_that_tick() {
    // The tick drops a gone value's copy, whose destructor enrols the same
    // parameter first. The enrolment is refused, and its tick has recorded
    // what was pending.
    let rig = rig(&["p"]);
    let p = rig.ids[0];
    let value = rig.value(0_i32);
    rig.history.enroll(&value, "v").expect("first");
    let enrols = rig.value(Enrols);
    rig.history.enroll(&enrols, "enrols").expect("first");
    // Only the history's copy is left, until the tick finds the value gone.
    drop(enrols);
    ENROL.with(|armed| *armed.borrow_mut() = Some((rig.history.clone(), p)));
    value.set(1);

    assert_eq!(
        rig.history.enroll_param(p),
        Err(HistoryError::AlreadyEnrolled)
    );
    let ran = ENROL.with(|armed| armed.borrow().is_none());
    assert!(ran, "the destructor ran");
    assert_eq!(rig.names(), ["v"], "the refused enrolment ticked");
    assert!(rig.ui.write(p, 8.0));
    rig.history.tick();
    assert_eq!(rig.names(), ["v", "p"], "the destructor's enrolment stands");
}

#[test]
fn a_worker_write_during_an_undos_tick_never_records_over_the_replay() {
    // For plain values: the write waits past the cut, the undo writes over
    // it, and it must not then record on top and drop the redo.
    let rig = rig(&[]);
    let probe = rig.value(Hooked(0));
    let value = rig.value(0_i32);
    rig.history.enroll(&probe, "probe").expect("first");
    rig.history.enroll(&value, "v").expect("first");
    value.set(1);
    rig.history.tick();
    probe.set(Hooked(0));
    let worker = value.clone();
    during_the_tick(move || worker.set(5));
    assert_eq!(rig.history.undo(), Ok(true));
    assert_eq!(value.get(), 0);
    rig.history.tick();
    assert_eq!(rig.history.redo(), Ok(true), "the redo survived");
    assert_eq!(value.get(), 1);
}

#[test]
fn a_refused_undo_records_nothing() {
    // A gesture open refuses before the tick.
    let rig = rig(&["p"]);
    let p = rig.ids[0];
    let value = rig.value(0_i32);
    rig.history.enroll(&value, "v").expect("first");
    value.set(1);
    let drag = rig.ui.begin(p);
    assert_eq!(rig.history.undo(), Err(HistoryError::GestureOpen));
    assert_eq!(rig.history.jump(0), Err(HistoryError::GestureOpen));
    assert!(rig.history.is_empty());
    drop(drag);
    assert_eq!(rig.history.undo(), Ok(true));
    assert_eq!(value.get(), 0);
}

#[test]
fn clear_records_what_changed_first_so_nothing_from_before_it_lists_after() {
    let rig = rig(&[]);
    let value = rig.value(0_i32);
    rig.history.enroll(&value, "v").expect("first");
    value.set(1);
    rig.history.clear().expect("idle");
    rig.history.tick();
    assert!(rig.history.is_empty());
    value.set(2);
    rig.history.tick();
    rig.history.undo().expect("idle");
    assert_eq!(value.get(), 1, "the baseline moved before the clear");
}

#[test]
fn a_plain_value_gesture_records_from_its_first_open_after_what_came_before() {
    // Writes before the gesture flush first; a nested open keeps the first
    // before-value; an unrecorded write flushes first too.
    let rig = rig(&[]);
    let value = rig.value(1_i32);
    rig.history.enroll(&value, "v").expect("first");
    value.set(2);
    let outer = value.begin();
    outer.set(3);
    let inner = value.begin();
    inner.set(4);
    drop(inner);
    drop(outer);
    value.set(6);
    {
        let _applying = rig.writer.applying();
        value.set(7);
    }
    rig.history.tick();
    assert_eq!(rig.history.len(), 3);
    rig.history.undo().expect("idle");
    assert_eq!(value.get(), 4, "the write before the apply");
    rig.history.undo().expect("idle");
    assert_eq!(value.get(), 2, "the gesture, from its first open");
    rig.history.undo().expect("idle");
    assert_eq!(value.get(), 1);
}

#[test]
fn a_plain_value_gesture_back_to_its_start_records_nothing_and_still_moves_the_baseline() {
    let rig = rig(&[]);
    let value = rig.value(1_i32);
    rig.history.enroll(&value, "v").expect("first");
    let drag = value.begin();
    drag.set(3);
    drag.set(1);
    drop(drag);
    rig.history.tick();
    assert!(rig.history.is_empty());

    let drag = value.begin();
    drag.set(3);
    drop(drag);
    rig.history.tick();
    value.set(5);
    rig.history.tick();
    rig.history.undo().expect("idle");
    assert_eq!(value.get(), 3, "the close moved the baseline");
}

#[test]
fn an_untracked_value_queues_nothing() {
    // A value never enrolled, or forgotten, keeps no copies of its writes.
    let rig = rig(&[]);
    let count = Arc::new(());
    let value = rig.value(Counted::new(&count, 0));
    for n in 0..10 {
        value.set(Counted::new(&count, n));
        let drag = value.begin();
        drag.set(Counted::new(&count, n + 1));
        drop(drag);
    }
    assert_eq!(Arc::strong_count(&count), 2, "the value's own copy only");
    rig.history.enroll(&value, "v").expect("first");
    assert!(rig.history.forget(&value));
    value.set(Counted::new(&count, 50));
    assert_eq!(Arc::strong_count(&count), 2);
}

/// One step of the UI thread, a host write, or a tick.
#[derive(Debug, Clone)]
enum Op {
    Open { param: usize },
    Close { token: usize },
    Write { token: usize, value: u8 },
    Lone { param: usize, value: u8 },
    Host { param: usize, value: u8 },
    Edit { param: usize, value: u8 },
    Sync { param: usize, value: u8 },
    Tick,
}

fn op() -> impl Strategy<Value = Op> {
    let value = 0_u8..=10;
    prop_oneof![
        (0_usize..2).prop_map(|param| Op::Open { param }),
        (0_usize..3).prop_map(|token| Op::Close { token }),
        (0_usize..3, value.clone()).prop_map(|(token, value)| Op::Write { token, value }),
        (0_usize..2, value.clone()).prop_map(|(param, value)| Op::Lone { param, value }),
        (0_usize..2, value.clone()).prop_map(|(param, value)| Op::Host { param, value }),
        (0_usize..2, value.clone()).prop_map(|(param, value)| Op::Edit { param, value }),
        (0_usize..2, value).prop_map(|(param, value)| Op::Sync { param, value }),
        Just(Op::Tick),
    ]
}

/// A parameter gesture as the model tracks it: tokens open, the value the
/// gesture shows, its before-value and the user's last value.
#[derive(Debug, Clone, Copy, Default)]
struct Open {
    tokens: u32,
    shown: f32,
    before: Option<f32>,
    after: Option<f32>,
}

proptest! {
    #![proptest_config(ProptestConfig {
        failure_persistence: Some(Box::new(FileFailurePersistence::Direct(
            "tests/enroll.proptest-regressions",
        ))),
        ..ProptestConfig::default()
    })]

    /// Each step is one user gesture with a net change, which the host never
    /// writes.
    ///
    /// With host writes, program edits, syncs and gestures interleaved, each
    /// step runs from its before-value to the user's last value, and no
    /// host-written value is ever a step's value unless the user wrote it too.
    /// Undoing every step walks the user's values back.
    #[test]
    fn each_step_is_one_user_gesture_and_the_host_never_writes_one(
        ops in proptest::collection::vec(op(), 0..60),
    ) {
        let rig = rig(&["a", "b"]);
        rig.history.enroll_params();
        let audio = rig.set.audio();
        let program = rig.set.program();
        let mut tokens: Vec<(usize, seam_params::Gesture)> = Vec::new();
        let mut open = [Open::default(); 2];
        // (param, before, after) for every gesture that closed changed.
        let mut expected: Vec<(usize, f32, f32)> = Vec::new();
        let close = |param: usize, open: &mut [Open; 2], expected: &mut Vec<(usize, f32, f32)>| {
            let state = &mut open[param];
            state.tokens -= 1;
            if state.tokens == 0
                && let (Some(before), Some(after)) = (state.before.take(), state.after.take())
                && before != after
            {
                expected.push((param, before, after));
            }
        };
        for op in &ops {
            match *op {
                Op::Open { param } => {
                    let state = &mut open[param];
                    if state.tokens == 0 {
                        state.shown = rig.set.value(rig.ids[param]);
                        state.before = None;
                        state.after = None;
                    }
                    state.tokens += 1;
                    tokens.push((param, rig.ui.begin(rig.ids[param])));
                }
                Op::Close { token } => {
                    if token < tokens.len() {
                        let (param, gesture) = tokens.remove(token);
                        drop(gesture);
                        close(param, &mut open, &mut expected);
                    }
                }
                Op::Write { token, value } => {
                    if let Some((param, gesture)) = tokens.get(token) {
                        let value = f32::from(value);
                        let state = &mut open[*param];
                        if gesture.write(value) {
                            state.before.get_or_insert(state.shown);
                            state.after = Some(value);
                            state.shown = value;
                        }
                    }
                }
                Op::Lone { param, value } => {
                    let value = f32::from(value);
                    let id = rig.ids[param];
                    let state = &mut open[param];
                    let shown = if state.tokens > 0 { state.shown } else { rig.set.value(id) };
                    if rig.ui.write(id, value) {
                        if state.tokens > 0 {
                            state.before.get_or_insert(shown);
                            state.after = Some(value);
                            state.shown = value;
                        } else {
                            expected.push((param, shown, value));
                        }
                    }
                }
                Op::Host { param, value } => {
                    audio.host_write(rig.ids[param], f64::from(value) / 10.0);
                }
                Op::Edit { param, value } => {
                    let id = rig.ids[param];
                    let state = &mut open[param];
                    let mut edit = rig.ui.edit();
                    if edit.write(id, f32::from(value)) && state.tokens > 0 {
                        state.shown = f32::from(value);
                    }
                }
                Op::Sync { param, value } => {
                    program.sync(rig.ids[param], f32::from(value));
                }
                Op::Tick => rig.history.tick(),
            }
        }
        while let Some((param, gesture)) = tokens.pop() {
            drop(gesture);
            close(param, &mut open, &mut expected);
        }
        rig.history.tick();
        prop_assert_eq!(rig.history.len(), expected.len());
        // Undo walks back through the steps newest first; each lands its
        // parameter on that step's before-value.
        for &(param, before, _) in expected.iter().rev() {
            prop_assert_eq!(rig.history.undo(), Ok(true));
            prop_assert_eq!(rig.set.value(rig.ids[param]), before);
        }
        for &(param, _, after) in &expected {
            prop_assert_eq!(rig.history.redo(), Ok(true));
            prop_assert_eq!(rig.set.value(rig.ids[param]), after);
        }
    }
}

/// One thing done to a plain value.
#[derive(Debug, Clone)]
enum ValueOp {
    Begin,
    End,
    Set(u8),
    Applied(u8),
    Tick,
}

fn value_op() -> impl Strategy<Value = ValueOp> {
    prop_oneof![
        Just(ValueOp::Begin),
        Just(ValueOp::End),
        (0_u8..4).prop_map(ValueOp::Set),
        (0_u8..4).prop_map(ValueOp::Set),
        (0_u8..4).prop_map(ValueOp::Applied),
        Just(ValueOp::Tick),
    ]
}

/// What the rules say a plain value records.
///
/// Writes outside a gesture coalesce until a tick or another event; a gesture
/// is one step from its open, or from a restore during it, to its last
/// recorded write; a write under the applying mark records nothing and moves
/// the baseline.
#[derive(Debug, Default)]
struct ValueModel {
    value: u8,
    baseline: u8,
    pending: Option<u8>,
    gestures: u32,
    before: u8,
    after: Option<u8>,
    steps: Vec<(u8, u8)>,
}

impl ValueModel {
    fn flush(&mut self) {
        if let Some(value) = self.pending.take() {
            if value != self.baseline {
                self.steps.push((self.baseline, value));
            }
            self.baseline = value;
        }
    }

    fn apply(&mut self, op: &ValueOp) {
        match *op {
            ValueOp::Begin => {
                if self.gestures == 0 {
                    self.before = self.value;
                    self.after = None;
                }
                self.gestures += 1;
            }
            ValueOp::End => {
                if self.gestures == 0 {
                    return;
                }
                self.gestures -= 1;
                if self.gestures == 0
                    && let Some(after) = self.after.take()
                {
                    self.flush();
                    if after != self.before {
                        self.steps.push((self.before, after));
                    }
                    self.baseline = after;
                }
            }
            ValueOp::Set(value) => {
                self.value = value;
                if self.gestures > 0 {
                    self.after = Some(value);
                } else {
                    self.pending = Some(value);
                }
            }
            ValueOp::Applied(value) => {
                self.value = value;
                if self.gestures > 0 {
                    self.before = value;
                    self.after = None;
                }
                self.flush();
                self.baseline = value;
            }
            ValueOp::Tick => self.flush(),
        }
    }
}

proptest! {
    #![proptest_config(ProptestConfig {
        failure_persistence: Some(Box::new(FileFailurePersistence::Direct(
            "tests/enroll.proptest-regressions",
        ))),
        ..ProptestConfig::default()
    })]

    /// Each step is one gesture with a
    /// net change, or one tick's coalesced writes, and undoing every step
    /// walks back through each before-value.
    #[test]
    fn a_plain_value_records_what_the_model_says(
        ops in proptest::collection::vec(value_op(), 0..60),
    ) {
        let rig = rig(&[]);
        let value = rig.value(0_u8);
        rig.history.enroll(&value, "v").expect("first");
        let mut model = ValueModel::default();
        let mut tokens = Vec::new();
        for op in &ops {
            match *op {
                ValueOp::Begin => tokens.push(value.begin()),
                ValueOp::End => {
                    tokens.pop();
                }
                ValueOp::Set(v) => value.set(v),
                ValueOp::Applied(v) => {
                    let _applying = rig.writer.applying();
                    value.set(v);
                }
                ValueOp::Tick => rig.history.tick(),
            }
            model.apply(op);
        }
        while tokens.pop().is_some() {
            model.apply(&ValueOp::End);
        }
        rig.history.tick();
        model.apply(&ValueOp::Tick);
        prop_assert_eq!(rig.history.len(), model.steps.len());
        for &(before, _) in model.steps.iter().rev() {
            prop_assert_eq!(rig.history.undo(), Ok(true));
            prop_assert_eq!(value.get(), before);
        }
        for &(_, after) in &model.steps {
            prop_assert_eq!(rig.history.redo(), Ok(true));
            prop_assert_eq!(value.get(), after);
        }
    }
}

#[test]
fn a_value_and_a_list_start_with_the_restore_origin() {
    // Nothing derived from an initial value records, nor from a
    // parameter's default (ParamSet::read).
    let rig = rig(&[]);
    let reading = rig.value(1_i32).read();
    assert_eq!((reading.changes, reading.origin), (0, Origin::Restore));
    let list = List::new(vec![1_u8], &rig.set);
    let reading = list.read(<[_]>::len);
    assert_eq!((reading.changes, reading.origin), (0, Origin::Restore));
}
