//! User, program and replay writes, gestures, and what they tell the host.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use proptest::prelude::*;
use proptest::test_runner::FileFailurePersistence;
use seam_params::{
    GestureRecords, HostEvent, HostEventDrain, Kind, Origin, ParamDecl, ParamId, ParamSet,
    ParamSetBuilder, UiParams,
};
use seam_testkit::sync::Latch;

fn param(key: &'static str) -> ParamDecl {
    ParamDecl::new(key, key, Kind::continuous(0.0, 10.0, 1.0, None), 5.0)
}

fn set_of(keys: &[&'static str]) -> Arc<ParamSet> {
    let mut builder = ParamSetBuilder::new();
    for &key in keys {
        builder.add(param(key)).expect("valid");
    }
    builder.build()
}

fn ui_of(set: &Arc<ParamSet>) -> UiParams {
    set.take_ui().expect("the first UI handle")
}

/// A UI handle and its one gesture record drain.
fn ui_and_records(set: &Arc<ParamSet>) -> (UiParams, GestureRecords) {
    let ui = ui_of(set);
    let records = ui.take_gesture_records().expect("the first drain");
    (ui, records)
}

/// The records taken, as (parameter, before, after).
fn steps(records: &GestureRecords) -> Vec<(ParamId, f32, f32)> {
    records
        .take()
        .into_iter()
        .map(|record| (record.id, record.before, record.after))
        .collect()
}

fn drained(drain: &mut HostEventDrain) -> Vec<HostEvent> {
    let mut events = Vec::new();
    drain.drain(|event| events.push(event));
    events
}

fn value(id: ParamId, real: f32) -> HostEvent {
    HostEvent::Value {
        id,
        normalized: f64::from(real) / 10.0,
    }
}

fn step(id: ParamId, before: f32, after: f32) -> (ParamId, f32, f32) {
    (id, before, after)
}

#[test]
fn a_lone_write_is_one_complete_gesture_and_one_step() {
    let set = set_of(&["p"]);
    let (ui, records) = ui_and_records(&set);
    let mut host = set.take_host_event_drain().expect("the first drain");
    let p = set.id("p").expect("declared");

    assert!(ui.write(p, 7.2));
    assert_eq!(set.value(p), 7.0);
    assert_eq!(set.changes(p), 1);
    assert_eq!(
        drained(&mut host),
        [HostEvent::Begin(p), value(p, 7.0), HostEvent::End(p)]
    );
    assert_eq!(steps(&records), [step(p, 5.0, 7.0)]);
    assert!(records.take().is_empty(), "taken once");
}

#[test]
fn a_click_that_changes_nothing_does_nothing() {
    let set = set_of(&["p"]);
    let (ui, records) = ui_and_records(&set);
    let mut host = set.take_host_event_drain().expect("drain");
    let p = set.id("p").expect("declared");

    // 5.3 snaps to the 5 already stored.
    assert!(!ui.would_change(p, 5.3));
    assert!(!ui.write(p, 5.3));
    // Not finite: refused.
    assert!(!ui.write(p, f32::NAN));
    assert_eq!(set.changes(p), 0);
    assert!(drained(&mut host).is_empty());
    assert!(records.take().is_empty());
}

#[test]
fn a_drag_is_one_bracket_and_one_step_from_the_pre_drag_value() {
    let set = set_of(&["p"]);
    let (ui, records) = ui_and_records(&set);
    let mut host = set.take_host_event_drain().expect("drain");
    let p = set.id("p").expect("declared");

    let drag = ui.begin(p);
    assert!(ui.is_open(p));
    assert!(drag.write(6.0));
    assert_eq!(drained(&mut host), [HostEvent::Begin(p), value(p, 6.0)]);
    assert!(drag.write(7.0));
    assert!(drag.write(8.0));
    // Values coalesce to the latest between two drains.
    assert_eq!(drained(&mut host), [value(p, 8.0)]);
    drop(drag);
    assert!(!ui.is_open(p));
    assert_eq!(drained(&mut host), [HostEvent::End(p)]);
    assert_eq!(steps(&records), [step(p, 5.0, 8.0)]);
}

#[test]
fn a_touch_reaches_the_host_before_anything_changes() {
    // A touch held still opens the host's bracket, so Touch and Latch modes
    // stop playing automation, while the open gesture holds the surfaces.
    let set = set_of(&["p"]);
    let (ui, records) = ui_and_records(&set);
    let audio = set.audio();
    let mut host = set.take_host_event_drain().expect("drain");
    let p = set.id("p").expect("declared");

    let touch = ui.begin(p);
    assert_eq!(drained(&mut host), [HostEvent::Begin(p)]);
    audio.host_write(p, 0.9);
    assert_eq!(set.value(p), 9.0);
    assert_eq!(
        ui.shown(p),
        Some(5.0),
        "surfaces show the touch, not the host"
    );
    drop(touch);
    assert_eq!(ui.shown(p), None);
    assert_eq!(drained(&mut host), [HostEvent::End(p)]);
    assert!(records.take().is_empty(), "nothing changed, so no step");
}

#[test]
fn nested_gestures_collapse_to_one_bracket_and_one_step() {
    let set = set_of(&["p"]);
    let (ui, records) = ui_and_records(&set);
    let mut host = set.take_host_event_drain().expect("drain");
    let p = set.id("p").expect("declared");

    let knob = ui.begin(p);
    let slider = ui.begin(p);
    assert!(knob.write(6.0));
    assert!(slider.write(9.0));
    drop(knob);
    assert!(ui.is_open(p), "the slider still holds it");
    assert!(records.take().is_empty());
    drop(slider);
    assert_eq!(
        drained(&mut host),
        [HostEvent::Begin(p), value(p, 9.0), HostEvent::End(p)]
    );
    assert_eq!(records.take().len(), 1);
}

#[test]
fn a_drag_back_to_its_start_makes_no_step() {
    let set = set_of(&["p"]);
    let (ui, records) = ui_and_records(&set);
    let mut host = set.take_host_event_drain().expect("drain");
    let p = set.id("p").expect("declared");

    let drag = ui.begin(p);
    assert!(drag.write(6.0));
    assert!(drag.write(5.0));
    drop(drag);
    assert_eq!(
        drained(&mut host),
        [HostEvent::Begin(p), value(p, 5.0), HostEvent::End(p)]
    );
    assert!(records.take().is_empty());
}

#[test]
fn a_user_write_compares_with_the_value_the_gesture_shows() {
    // A user write equal to a host write not yet shown is still a change, so
    // undo's after-value includes it.
    let set = set_of(&["p"]);
    let (ui, records) = ui_and_records(&set);
    let audio = set.audio();
    let p = set.id("p").expect("declared");

    let drag = ui.begin(p);
    assert!(drag.write(6.0));
    audio.host_write(p, 0.8);
    assert_eq!(set.value(p), 8.0);
    assert_eq!(ui.shown(p), Some(6.0));
    assert!(drag.write(8.0), "a change from the drag's 6");
    drop(drag);
    assert_eq!(steps(&records), [step(p, 5.0, 8.0)]);
}

#[test]
fn a_user_write_inside_a_program_edit_starts_from_the_edits_value() {
    // A user's step must not include the plugin's own edit.
    let set = set_of(&["p"]);
    let (ui, records) = ui_and_records(&set);
    let p = set.id("p").expect("declared");

    let mut edit = ui.edit();
    assert!(edit.write(p, 9.0));
    let click = ui.begin(p);
    assert!(!click.write(9.0), "the edit's value is already shown");
    let changes = set.changes(p);
    assert!(click.write(5.0), "back to the pre-edit value is a change");
    assert_eq!(set.changes(p), changes + 1);
    drop(click);
    drop(edit);
    assert_eq!(steps(&records), [step(p, 9.0, 5.0)]);
}

#[test]
fn the_close_writes_nothing_so_a_host_write_during_the_drag_stands() {
    let set = set_of(&["p"]);
    let (ui, records) = ui_and_records(&set);
    let audio = set.audio();
    let p = set.id("p").expect("declared");

    let drag = ui.begin(p);
    assert!(drag.write(6.0));
    audio.host_write(p, 0.2);
    let before_close = set.changes(p);
    drop(drag);
    assert_eq!(set.value(p), 2.0);
    assert_eq!(set.changes(p), before_close);
    assert_eq!(records.take()[0].after, 6.0);
}

#[test]
fn a_step_carries_the_restore_stamp_at_its_close() {
    // Undo drops a step recorded before a restore it has not seen.
    let set = set_of(&["p"]);
    let clock = std::sync::Arc::clone(set.clock());
    let ui = set.take_ui().expect("UI");
    let drain = ui.take_gesture_records().expect("the first drain");
    let p = set.id("p").expect("declared");

    clock.writer().stored();
    assert!(ui.write(p, 7.0));
    let records = drain.take();
    assert_eq!(records[0].stamp.stored(), 1);
    assert_eq!(records[0].stamp, clock.reader().stamp());
}

#[test]
fn undo_can_tell_when_a_user_gesture_is_open() {
    // Undo refuses to replay while a user gesture is open.
    let set = set_of(&["a", "b"]);
    let ui = ui_of(&set);
    let (a, b) = (set.id("a").expect("a"), set.id("b").expect("b"));

    assert!(!ui.user_gesture_open());
    let drag = ui.begin(a);
    assert!(ui.user_gesture_open());
    drop(drag);
    assert!(!ui.user_gesture_open());

    // A program edit is not a user gesture.
    let mut edit = ui.edit();
    assert!(edit.write(b, 1.0));
    assert!(!ui.user_gesture_open());
}

#[test]
fn a_program_edit_tells_the_host_and_records_nothing() {
    let set = set_of(&["a", "b"]);
    let (ui, records) = ui_and_records(&set);
    let mut host = set.take_host_event_drain().expect("drain");
    let (a, b) = (set.id("a").expect("a"), set.id("b").expect("b"));

    let mut edit = ui.edit();
    assert!(edit.write(a, 1.0));
    assert!(edit.write(b, 2.0));
    assert!(edit.write(a, 3.0));
    assert!(!edit.write(b, 2.0), "unchanged");
    assert!(ui.is_open(a) && ui.is_open(b));
    drop(edit);
    assert_eq!(
        drained(&mut host),
        [
            HostEvent::Begin(a),
            value(a, 3.0),
            HostEvent::End(a),
            HostEvent::Begin(b),
            value(b, 2.0),
            HostEvent::End(b),
        ]
    );
    assert!(records.take().is_empty());
}

#[test]
fn a_program_edit_during_a_drag_shares_the_drags_bracket() {
    let set = set_of(&["p"]);
    let (ui, records) = ui_and_records(&set);
    let mut host = set.take_host_event_drain().expect("drain");
    let p = set.id("p").expect("declared");

    let drag = ui.begin(p);
    assert!(drag.write(6.0));
    let mut edit = ui.edit();
    assert!(edit.write(p, 9.0));
    drop(edit);
    assert!(ui.is_open(p));
    assert_eq!(drained(&mut host), [HostEvent::Begin(p), value(p, 9.0)]);
    drop(drag);
    assert_eq!(drained(&mut host), [HostEvent::End(p)]);
    // Only the user's own writes make the step.
    assert_eq!(steps(&records), [step(p, 5.0, 6.0)]);
}

#[test]
fn a_replay_is_one_complete_host_gesture_and_never_a_step() {
    let set = set_of(&["p"]);
    let (ui, records) = ui_and_records(&set);
    let mut host = set.take_host_event_drain().expect("drain");
    let p = set.id("p").expect("declared");

    assert!(ui.replay(p, 2.0));
    assert!(!ui.replay(p, 2.0));
    assert_eq!(
        drained(&mut host),
        [HostEvent::Begin(p), value(p, 2.0), HostEvent::End(p)]
    );
    assert!(records.take().is_empty());
}

#[test]
fn a_sync_from_another_thread_asks_for_a_rescan_and_opens_nothing() {
    let set = set_of(&["p"]);
    let (ui, records) = ui_and_records(&set);
    let mut host = set.take_host_event_drain().expect("drain");
    let p = set.id("p").expect("declared");
    let program = set.program();

    let changed = std::thread::spawn(move || (program.sync(p, 3.0), program.sync(p, 3.0)))
        .join()
        .expect("the worker");
    assert_eq!(changed, (true, false));
    assert_eq!(set.value(p), 3.0);
    assert_eq!(set.changes(p), 1);
    assert!(set.take_rescan());
    assert!(!set.take_rescan(), "taken once");
    assert!(drained(&mut host).is_empty());
    assert!(!ui.is_open(p));
    assert!(records.take().is_empty());
    assert!(!set.program().sync(p, f32::NAN), "refused");
}

#[test]
fn there_is_one_ui_handle_and_one_drain_at_a_time() {
    let set = set_of(&["p"]);
    let ui = ui_of(&set);
    let clone = ui.clone();
    assert!(set.take_ui().is_none());
    drop(ui);
    assert!(set.take_ui().is_none(), "a clone is still alive");
    drop(clone);
    assert!(set.take_ui().is_some());

    let host = set.take_host_event_drain().expect("drain");
    assert!(set.take_host_event_drain().is_none());
    drop(host);
    assert!(set.take_host_event_drain().is_some());

    // One gesture record drain per handle, and records queue only
    // while one is alive.
    let ui = ui_of(&set);
    let p = set.id("p").expect("declared");
    assert!(ui.write(p, 7.0));
    let records = ui.take_gesture_records().expect("the first drain");
    assert!(records.take().is_empty(), "nothing queued with no drain");
    assert!(ui.clone().take_gesture_records().is_none());
    assert!(ui.write(p, 8.0));
    drop(records);
    let records = ui.take_gesture_records().expect("released");
    assert!(
        records.take().is_empty(),
        "a dropped drain drops its records"
    );
}

#[test]
fn every_write_leaves_its_origin_beside_the_change_count() {
    // A value derived from a parameter passes its origin on.
    let set = set_of(&["p"]);
    let ui = set.take_ui().expect("UI");
    let p = set.id("p").expect("declared");
    let read = |set: &ParamSet| {
        let reading = set.read(p);
        (reading.changes, reading.value, reading.origin)
    };
    assert_eq!(read(&set), (0, 5.0, Origin::Restore), "the default");

    assert!(ui.write(p, 7.0));
    assert_eq!(read(&set), (1, 7.0, Origin::User));
    let mut edit = ui.edit();
    assert!(edit.write(p, 6.0));
    drop(edit);
    assert_eq!(set.read(p).origin, Origin::Program);
    assert!(ui.replay(p, 5.0));
    assert_eq!(set.read(p).origin, Origin::Replay);
    set.audio().host_write(p, 0.9);
    assert_eq!(set.read(p).origin, Origin::Host);
    set.audio().learned_write(p, 0.1);
    assert_eq!(set.read(p).origin, Origin::Learn);
    assert!(set.program().sync(p, 4.0));
    assert_eq!(set.read(p).origin, Origin::Program);
    set.program().load().restore(p, 3.0);
    // What a derived value reads: the value with the origin of its write.
    assert_eq!(read(&set), (7, 3.0, Origin::Restore));
}

#[test]
fn a_gesture_reopened_between_drains_merges_into_the_open_bracket() {
    let set = set_of(&["p"]);
    let (ui, records) = ui_and_records(&set);
    let mut host = set.take_host_event_drain().expect("drain");
    let p = set.id("p").expect("declared");

    let first = ui.begin(p);
    assert!(first.write(6.0));
    assert_eq!(drained(&mut host), [HostEvent::Begin(p), value(p, 6.0)]);
    drop(first);
    let second = ui.begin(p);
    assert!(second.write(7.0));
    // The host never saw the first close, so it sees one bracket.
    assert_eq!(drained(&mut host), [value(p, 7.0)]);
    drop(second);
    assert_eq!(drained(&mut host), [HostEvent::End(p)]);
    // Undo still has two steps: the gestures were the user's.
    assert_eq!(records.take().len(), 2);
}

#[test]
fn a_drain_made_mid_gesture_opens_its_own_bracket() {
    let set = set_of(&["p"]);
    let ui = ui_of(&set);
    let p = set.id("p").expect("declared");

    let drag = ui.begin(p);
    assert!(drag.write(6.0));
    let mut host = set.take_host_event_drain().expect("drain");
    assert_eq!(drained(&mut host), [HostEvent::Begin(p)]);
    drop(drag);
    assert_eq!(drained(&mut host), [HostEvent::End(p)]);
}

#[test]
fn finishing_a_drain_ends_every_bracket_the_host_holds() {
    // A CLAP deactivate or a VST3 disconnect mid-drag.
    let set = set_of(&["p"]);
    let ui = ui_of(&set);
    let p = set.id("p").expect("declared");

    let drag = ui.begin(p);
    assert!(drag.write(6.0));
    let mut host = set.take_host_event_drain().expect("drain");
    assert_eq!(drained(&mut host), [HostEvent::Begin(p)]);
    assert!(drag.write(7.0));
    let mut events = Vec::new();
    host.finish(|event| events.push(event));
    assert_eq!(events, [value(p, 7.0), HostEvent::End(p)]);
    drop(host);

    // A drain made while the drag still holds it opens a fresh bracket.
    let mut host = set.take_host_event_drain().expect("a new drain");
    assert_eq!(drained(&mut host), [HostEvent::Begin(p)]);
    drop(drag);
    assert_eq!(drained(&mut host), [HostEvent::End(p)]);
}

#[test]
fn a_panicking_emit_leaves_the_rest_for_the_next_drain() {
    // Two lone writes make six events: a begin, a value and an end each.
    // Whichever event the emit panics on, the host has heard the ones before
    // it, and the next drain hands out that one and everything after it.
    let whole = {
        let set = set_of(&["a", "b"]);
        let ui = ui_of(&set);
        let mut host = set.take_host_event_drain().expect("drain");
        for id in set.ids() {
            assert!(ui.write(id, 6.0));
        }
        drained(&mut host)
    };
    assert_eq!(whole.len(), 6);
    for panic_at in 0..whole.len() {
        let set = set_of(&["a", "b"]);
        let ui = ui_of(&set);
        let mut host = set.take_host_event_drain().expect("drain");
        for id in set.ids() {
            assert!(ui.write(id, 6.0));
        }
        let mut heard = Vec::new();
        let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            host.drain(|event| {
                if heard.len() == panic_at {
                    panic!("emit panicked on event {panic_at}");
                }
                heard.push(event);
            });
        }));
        assert!(outcome.is_err());
        host.drain(|event| heard.push(event));
        assert_eq!(heard, whole, "panic at event {panic_at}");
        host.finish(|event| panic!("a bracket was left open: {event:?}"));
    }
}

#[test]
fn a_panicking_emit_keeps_a_gestures_end_for_the_next_drain() {
    let set = set_of(&["p"]);
    let ui = ui_of(&set);
    let p = set.id("p").expect("declared");
    let mut host = set.take_host_event_drain().expect("drain");
    let drag = ui.begin(p);
    assert!(drag.write(6.0));
    assert_eq!(drained(&mut host), [HostEvent::Begin(p), value(p, 6.0)]);
    drop(drag);
    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        host.drain(|_| panic!("emit panicked on the end"));
    }));
    assert!(outcome.is_err());
    assert_eq!(drained(&mut host), [HostEvent::End(p)]);
}

#[test]
fn a_finish_whose_end_panics_keeps_the_bracket_open_for_the_next_finish() {
    // The host stops listening with a gesture open, and telling it the End
    // panics: the bracket stays open, so the next finish ends it, and the
    // host never holds a begin with no end.
    let set = set_of(&["p"]);
    let ui = ui_of(&set);
    let p = set.id("p").expect("declared");
    let mut host = set.take_host_event_drain().expect("drain");
    let drag = ui.begin(p);
    assert!(drag.write(6.0));
    assert_eq!(drained(&mut host), [HostEvent::Begin(p), value(p, 6.0)]);
    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        host.finish(|_| panic!("emit panicked on the end"));
    }));
    assert!(outcome.is_err());
    let mut heard = Vec::new();
    host.finish(|event| heard.push(event));
    assert_eq!(heard, [HostEvent::End(p)]);
    drop(drag);
}

/// Checks a stream of events: balanced, non-overlapping
/// pairs per parameter, values only inside them, every bracket closed, and
/// the last value told for each parameter the store's when the store moved.
fn check_balanced(set: &ParamSet, events: &[HostEvent]) -> Result<(), String> {
    let mut open: HashMap<ParamId, bool> = HashMap::new();
    let mut last: HashMap<ParamId, f64> = HashMap::new();
    for event in events {
        match *event {
            HostEvent::Begin(id) => {
                if open.insert(id, true) == Some(true) {
                    return Err(format!("a second begin for {id:?}"));
                }
            }
            HostEvent::Value { id, normalized } => {
                if open.get(&id) != Some(&true) {
                    return Err(format!("a value outside a bracket for {id:?}"));
                }
                last.insert(id, normalized);
            }
            HostEvent::End(id) => {
                if open.insert(id, false) != Some(true) {
                    return Err(format!("an end with no begin for {id:?}"));
                }
            }
        }
    }
    if open.values().any(|&o| o) {
        return Err("a bracket left open".to_owned());
    }
    for id in set.ids() {
        let moved = set.value(id) != set.decl(id).default_value();
        match last.get(&id) {
            Some(&normalized) if normalized != set.normalized(id) => {
                return Err(format!(
                    "{id:?} was last told {normalized}, the store is elsewhere"
                ));
            }
            None if moved => return Err(format!("{id:?} moved and the host was never told")),
            _ => {}
        }
    }
    Ok(())
}

/// One step of the UI thread, or a drain.
#[derive(Debug, Clone)]
enum Op {
    Open { param: usize },
    Close { token: usize },
    Write { token: usize, value: u8 },
    Lone { param: usize, value: u8 },
    Replay { param: usize, value: u8 },
    EditOpen,
    EditWrite { param: usize, value: u8 },
    EditClose,
    Drain,
}

fn op() -> impl Strategy<Value = Op> {
    prop_oneof![
        (0_usize..3).prop_map(|param| Op::Open { param }),
        (0_usize..8).prop_map(|token| Op::Close { token }),
        (0_usize..8, 0_u8..=10).prop_map(|(token, value)| Op::Write { token, value }),
        (0_usize..3, 0_u8..=10).prop_map(|(param, value)| Op::Lone { param, value }),
        (0_usize..3, 0_u8..=10).prop_map(|(param, value)| Op::Replay { param, value }),
        Just(Op::EditOpen),
        (0_usize..3, 0_u8..=10).prop_map(|(param, value)| Op::EditWrite { param, value }),
        Just(Op::EditClose),
        Just(Op::Drain),
    ]
}

/// Runs the UI side of `ops`, handing each drain to `drain`.
fn run(ui: &UiParams, ids: &[ParamId], ops: &[Op], mut drain: impl FnMut()) {
    let mut tokens = Vec::new();
    let mut edit = None;
    for op in ops {
        match *op {
            Op::Open { param } => tokens.push(ui.begin(ids[param])),
            Op::Close { token } => {
                if token < tokens.len() {
                    drop(tokens.remove(token));
                }
            }
            Op::Write { token, value } => {
                if let Some(gesture) = tokens.get(token) {
                    gesture.write(f32::from(value));
                }
            }
            Op::Lone { param, value } => {
                ui.write(ids[param], f32::from(value));
            }
            Op::Replay { param, value } => {
                ui.replay(ids[param], f32::from(value));
            }
            Op::EditOpen => edit = edit.or_else(|| Some(ui.edit())),
            Op::EditWrite { param, value } => {
                if let Some(edit) = edit.as_mut() {
                    edit.write(ids[param], f32::from(value));
                }
            }
            Op::EditClose => edit = None,
            Op::Drain => drain(),
        }
    }
}

proptest! {
    #![proptest_config(ProptestConfig {
        failure_persistence: Some(Box::new(FileFailurePersistence::Direct(
            "tests/writes.proptest-regressions",
        ))),
        ..ProptestConfig::default()
    })]

    /// Any interleaving of begins and ends on several controls, with program
    /// edits and replays among them, reaches the host as balanced,
    /// non-overlapping begin and end pairs per parameter.
    #[test]
    fn the_host_sees_balanced_brackets(ops in proptest::collection::vec(op(), 0..60)) {
        let set = set_of(&["a", "b", "c"]);
        let ui = ui_of(&set);
        let host = std::cell::RefCell::new(set.take_host_event_drain().expect("drain"));
        let ids: Vec<ParamId> = set.ids().collect();
        let events = std::cell::RefCell::new(Vec::new());
        run(&ui, &ids, &ops, || host.borrow_mut().drain(|event| events.borrow_mut().push(event)));
        host.borrow_mut().drain(|event| events.borrow_mut().push(event));
        let result = check_balanced(&set, &events.borrow());
        prop_assert!(result.is_ok(), "{}", result.unwrap_err());
    }
}

#[test]
fn the_host_sees_balanced_brackets_while_a_real_thread_drains() {
    // The UI thread runs a fixed pseudo-random stream of operations while
    // another thread drains as fast as it can, as CLAP's audio thread does.
    let set = set_of(&["a", "b", "c"]);
    let ui = ui_of(&set);
    let ids: Vec<ParamId> = set.ids().collect();
    let mut host = set.take_host_event_drain().expect("drain");
    let started = Arc::new(Latch::new(1));
    let done = Arc::new(AtomicBool::new(false));
    let drainer = {
        let (started, done) = (Arc::clone(&started), Arc::clone(&done));
        std::thread::spawn(move || {
            let mut events = Vec::new();
            started.count_down();
            while !done.load(Ordering::Acquire) {
                host.drain(|event| events.push(event));
            }
            host.drain(|event| events.push(event));
            events
        })
    };
    started.wait();

    let mut seed = 0x9E37_79B9_u32;
    let mut next = |n: u32| {
        seed ^= seed << 13;
        seed ^= seed >> 17;
        seed ^= seed << 5;
        (seed % n) as u8
    };
    let ops: Vec<Op> = (0..50_000)
        .map(|_| match next(7) {
            0 => Op::Open {
                param: usize::from(next(3)),
            },
            1 => Op::Close {
                token: usize::from(next(4)),
            },
            2 | 3 => Op::Write {
                token: usize::from(next(4)),
                value: next(11),
            },
            4 => Op::Lone {
                param: usize::from(next(3)),
                value: next(11),
            },
            5 => Op::EditWrite {
                param: usize::from(next(3)),
                value: next(11),
            },
            _ => {
                if next(2) == 0 {
                    Op::EditOpen
                } else {
                    Op::EditClose
                }
            }
        })
        .collect();
    run(&ui, &ids, &ops, || {});

    done.store(true, Ordering::Release);
    let events = drainer.join().expect("the drainer");
    assert!(!events.is_empty());
    check_balanced(&set, &events).expect("balanced brackets");
}

#[test]
#[cfg(debug_assertions)]
fn a_user_write_while_a_restore_applies_is_refused_in_debug() {
    // A participant applying a restore writes through a sync.
    let set = set_of(&["p"]);
    let clock = std::sync::Arc::clone(set.clock());
    let ui = set.take_ui().expect("UI");
    let p = set.id("p").expect("declared");
    let writer = clock.writer();
    let _applying = writer.applying();
    seam_testkit::contract::expect_contract_panic("use ProgramParams::sync", || {
        ui.write(p, 7.0);
    });
}
