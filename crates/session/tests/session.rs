//! Registrations, saves, restores and the apply pass.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use seam_params::{ParamSetBuilder, RestoreReader};
use seam_session::{ApplyError, Participant, Section, Session, SessionError, SessionUi};
use serde_json::json;

/// A participant whose state is one string; empty saves nothing.
#[derive(Default)]
struct Notes {
    text: String,
    /// Every section an apply received.
    applied: Vec<Option<Section>>,
    captures: Cell<usize>,
    fail: bool,
    panic: bool,
    /// Runs inside the apply, after the state changed.
    during_apply: Option<Box<dyn FnMut()>>,
    /// Runs inside the capture.
    during_capture: Option<Box<dyn Fn()>>,
    /// Whether the clock carried the applying mark during the apply.
    saw_applying: Option<RestoreReader>,
    applying_seen: Vec<bool>,
}

impl Participant for Notes {
    fn capture(&self) -> Section {
        self.captures.set(self.captures.get() + 1);
        if let Some(during) = &self.during_capture {
            during();
        }
        if self.text.is_empty() {
            Section::Null
        } else {
            json!(self.text)
        }
    }

    fn apply(&mut self, section: Option<&Section>) -> Result<(), ApplyError> {
        self.applied.push(section.cloned());
        if let Some(clock) = &self.saw_applying {
            self.applying_seen.push(clock.applying());
        }
        if self.panic {
            panic!("a participant's apply panicked");
        }
        if self.fail {
            return Err(ApplyError::new("does not decode"));
        }
        self.text = match section {
            Some(Section::String(text)) => text.clone(),
            _ => "default".into(),
        };
        if let Some(during) = &mut self.during_apply {
            during();
        }
        Ok(())
    }
}

fn notes(text: &str) -> Rc<RefCell<Notes>> {
    Rc::new(RefCell::new(Notes {
        text: text.into(),
        ..Notes::default()
    }))
}

struct Rig {
    session: Session,
    ui: SessionUi,
    clock: RestoreReader,
}

fn rig() -> Rig {
    let set = ParamSetBuilder::new().build();
    let session = Session::new(&set);
    let ui = session.take_ui().expect("the first UI handle");
    Rig {
        session,
        ui,
        clock: set.clock().reader(),
    }
}

fn texts(participant: &Rc<RefCell<Notes>>) -> (String, Vec<Option<Section>>) {
    let participant = participant.borrow();
    (participant.text.clone(), participant.applied.clone())
}

#[test]
fn sections_compose_under_extra_and_nothing_to_save_leaves_the_key_out() {
    let rig = rig();
    assert_eq!(rig.session.save(), None, "nothing registered");
    let a = notes("x");
    let b = notes("");
    let _a = rig.ui.register("a", &a).expect("free");
    let _b = rig.ui.register("b", &b).expect("free");
    assert_eq!(rig.session.save(), Some(json!({"a": "x"})));
}

#[test]
fn a_save_runs_no_participant_code_and_a_change_recaptures() {
    // Captured at registration and on change, never by a save, from any
    // thread.
    let rig = rig();
    let a = notes("one");
    let registration = rig.ui.register("a", &a).expect("free");
    assert_eq!(a.borrow().captures.get(), 1);
    a.borrow_mut().text = "two".into();
    let session = rig.session.clone();
    let saved = std::thread::spawn(move || session.save())
        .join()
        .expect("the saving thread");
    assert_eq!(saved, Some(json!({"a": "one"})), "the last capture");
    assert_eq!(a.borrow().captures.get(), 1);
    registration.changed();
    assert_eq!(a.borrow().captures.get(), 2);
    assert_eq!(rig.session.save(), Some(json!({"a": "two"})));
}

#[test]
fn an_empty_key_a_taken_key_and_a_second_registration_are_refused() {
    let rig = rig();
    let a = notes("x");
    let b = notes("y");
    assert_eq!(
        rig.ui.register("", &a).map(|_| ()),
        Err(SessionError::EmptyKey)
    );
    let _a = rig.ui.register("a", &a).expect("free");
    assert_eq!(
        rig.ui.register("a", &b).map(|_| ()),
        Err(SessionError::KeyTaken("a".into()))
    );
    assert_eq!(
        rig.ui.register("other", &a).map(|_| ()),
        Err(SessionError::AlreadyRegistered)
    );
    assert_eq!(
        rig.session.save(),
        Some(json!({"a": "x"})),
        "nothing changed"
    );
    assert!(rig.session.take_ui().is_none(), "one UI handle per session");
}

#[test]
fn a_registration_and_its_session_end_in_either_order() {
    let rig = rig();
    let a = notes("x");
    let registration = rig.ui.register("a", &a).expect("free");
    assert!(registration.connected());
    let Rig { session, ui, .. } = rig;
    drop(session);
    drop(ui);
    assert!(!registration.connected());
    registration.changed();
    drop(registration);

    // The registration first: the participant leaves, and a restore no
    // longer reaches it.
    let rig = self::rig();
    let registration = rig.ui.register("a", &a).expect("free");
    drop(registration);
    rig.session.restore(Some(&json!({"a": "restored"})));
    rig.ui.tick();
    assert_eq!(texts(&a).0, "x");

    // The participant first: a pass skips it.
    let b = notes("y");
    let registration = rig.ui.register("b", &b).expect("free");
    drop(b);
    rig.session.restore(Some(&json!({"b": "restored"})));
    rig.ui.tick();
    assert!(registration.connected());
}

#[test]
fn a_restore_stores_every_section_at_once_and_applies_on_a_later_tick() {
    // From another thread, a save straight after returns what was
    // restored, before any participant applied.
    let rig = rig();
    let a = notes("a0");
    let b = notes("b0");
    let _a = rig.ui.register("a", &a).expect("free");
    let _b = rig.ui.register("b", &b).expect("free");
    let session = rig.session.clone();
    std::thread::spawn(move || session.restore(Some(&json!({"a": "a1", "b": "b1"}))))
        .join()
        .expect("the restoring thread");
    assert_eq!(rig.session.save(), Some(json!({"a": "a1", "b": "b1"})));
    assert!(
        a.borrow().applied.is_empty(),
        "applied on a tick, not before"
    );

    // On the UI thread too, the apply waits for the tick.
    rig.session.restore(Some(&json!({"a": "a2", "b": "b2"})));
    assert!(a.borrow().applied.is_empty());
    rig.ui.tick();
    assert_eq!(
        texts(&a),
        ("a2".into(), vec![Some(json!("a2"))]),
        "the newest, once"
    );
    assert_eq!(texts(&b), ("b2".into(), vec![Some(json!("b2"))]));
    rig.ui.tick();
    assert_eq!(a.borrow().applied.len(), 1, "nothing owed");
}

#[test]
fn a_missing_key_a_null_section_or_no_object_resets_to_defaults() {
    // A restore is a complete assignment; null means absent.
    for extra in [
        Some(json!({"other": 1})),
        Some(json!({"a": null})),
        Some(json!([1, 2])),
        None,
    ] {
        let rig = rig();
        let a = notes("x");
        let _a = rig.ui.register("a", &a).expect("free");
        rig.session.restore(extra.as_ref());
        assert_eq!(
            rig.session.save().and_then(|extra| extra.get("a").cloned()),
            None,
            "{extra:?}"
        );
        rig.ui.tick();
        assert_eq!(texts(&a), ("default".into(), vec![None]), "{extra:?}");
    }
}

#[test]
fn sections_no_participant_holds_are_saved_back_verbatim() {
    // A newer build's section survives.
    let rig = rig();
    let a = notes("x");
    let _a = rig.ui.register("a", &a).expect("free");
    let extra = json!({"a": "a1", "newer": {"deep": [1, 2, {"k": null}]}});
    rig.session.restore(Some(&extra));
    rig.ui.tick();
    assert_eq!(rig.session.save(), Some(extra));
    rig.session.restore(Some(&json!({"a": "a2"})));
    assert_eq!(
        rig.session.save(),
        Some(json!({"a": "a2"})),
        "replaced by the next restore"
    );
}

#[test]
fn a_failed_or_panicking_apply_keeps_its_state_and_raw_section_and_the_others_apply() {
    let rig = rig();
    let failing = notes("kept");
    failing.borrow_mut().fail = true;
    let panicking = notes("kept too");
    panicking.borrow_mut().panic = true;
    let fine = notes("old");
    let _f = rig.ui.register("failing", &failing).expect("free");
    let _p = rig.ui.register("panicking", &panicking).expect("free");
    let _o = rig.ui.register("fine", &fine).expect("free");
    let extra = json!({"failing": {"v": 2}, "panicking": [3], "fine": "new"});
    rig.session.restore(Some(&extra));
    rig.ui.tick();
    assert_eq!(texts(&failing).0, "kept");
    assert_eq!(texts(&panicking).0, "kept too");
    assert_eq!(texts(&fine).0, "new");
    assert_eq!(
        rig.session.save(),
        Some(extra),
        "the raw sections, unchanged"
    );
    assert_eq!(rig.clock.stamp().completed(), 1, "the pass still completed");
}

#[test]
fn a_participant_whose_apply_panicked_is_held_until_an_apply_succeeds() {
    // A panic can stop an apply halfway, so its captures are ignored and
    // its raw section stays saved until a later apply succeeds.
    let rig = rig();
    let a = notes("before");
    let registration = rig.ui.register("a", &a).expect("free");
    a.borrow_mut().panic = true;
    rig.session.restore(Some(&json!({"a": "restored"})));
    rig.ui.tick();
    assert_eq!(rig.session.save(), Some(json!({"a": "restored"})));
    a.borrow_mut().text = "half".into();
    registration.changed();
    rig.ui.tick();
    assert_eq!(
        rig.session.save(),
        Some(json!({"a": "restored"})),
        "the half-built state is never saved"
    );

    a.borrow_mut().panic = false;
    rig.session.restore(Some(&json!({"a": "again"})));
    rig.ui.tick();
    assert_eq!(texts(&a).0, "again");
    a.borrow_mut().text = "edited".into();
    registration.changed();
    assert_eq!(
        rig.session.save(),
        Some(json!({"a": "edited"})),
        "held no more"
    );
}

#[test]
fn a_failed_apply_does_not_hold_its_participant() {
    // A failed apply leaves the participant as it was.
    let rig = rig();
    let a = notes("before");
    let registration = rig.ui.register("a", &a).expect("free");
    a.borrow_mut().fail = true;
    rig.session.restore(Some(&json!({"a": {"v": 2}})));
    rig.ui.tick();
    assert_eq!(rig.session.save(), Some(json!({"a": {"v": 2}})));
    a.borrow_mut().text = "edited".into();
    registration.changed();
    assert_eq!(rig.session.save(), Some(json!({"a": "edited"})));
}

#[test]
fn a_failed_apply_leaves_a_participant_held_from_a_panic_still_held() {
    // A failed apply changes the participant nothing, so it cannot undo
    // what the panic before it left half done.
    let rig = rig();
    let a = notes("before");
    let registration = rig.ui.register("a", &a).expect("free");
    a.borrow_mut().panic = true;
    rig.session.restore(Some(&json!({"a": "restored"})));
    rig.ui.tick();
    a.borrow_mut().panic = false;
    a.borrow_mut().fail = true;
    rig.session.restore(Some(&json!({"a": "again"})));
    rig.ui.tick();
    a.borrow_mut().fail = false;
    a.borrow_mut().text = "half".into();
    registration.changed();
    assert_eq!(
        rig.session.save(),
        Some(json!({"a": "again"})),
        "still held after a failed apply"
    );
}

#[test]
fn the_pass_applies_in_registration_order_and_recaptures_each() {
    let rig = rig();
    let order = Rc::new(RefCell::new(Vec::new()));
    let mut held = Vec::new();
    let mut participants = Vec::new();
    for key in ["c", "a", "b"] {
        let participant = notes(key);
        let order = Rc::clone(&order);
        participant.borrow_mut().during_apply =
            Some(Box::new(move || order.borrow_mut().push(key)));
        held.push(rig.ui.register(key, &participant).expect("free"));
        participants.push(participant);
    }
    rig.session
        .restore(Some(&json!({"a": "A", "b": "B", "c": "C"})));
    rig.ui.tick();
    assert_eq!(*order.borrow(), ["c", "a", "b"]);
    for participant in &participants {
        assert_eq!(
            participant.borrow().captures.get(),
            2,
            "at registration and after the apply"
        );
    }
}

#[test]
fn a_pending_section_is_the_saved_one_until_its_apply_even_over_a_change() {
    let rig = rig();
    let a = notes("x");
    let registration = rig.ui.register("a", &a).expect("free");
    rig.session.restore(Some(&json!({"a": "restored"})));
    a.borrow_mut().text = "edited".into();
    registration.changed();
    assert_eq!(rig.session.save(), Some(json!({"a": "restored"})));
    assert_eq!(a.borrow().captures.get(), 1, "no capture while it waits");
    rig.ui.tick();
    a.borrow_mut().text = "edited".into();
    registration.changed();
    assert_eq!(
        rig.session.save(),
        Some(json!({"a": "edited"})),
        "after the apply"
    );
}

#[test]
fn a_restore_landing_during_a_capture_wins_over_it() {
    // An apply that lands during a capture cannot commit until the capture
    // ends, because the capture holds the participant. The stamp check
    // behind that is a second guard no test can reach.
    let rig = rig();
    let a = notes("x");
    let registration = rig.ui.register("a", &a).expect("free");
    let session = rig.session.clone();
    let ui = rig.ui.clone();
    a.borrow_mut().during_capture = Some(Box::new(move || {
        session.restore(Some(&json!({"a": "restored"})));
        // The capture holds the participant, so the apply waits.
        ui.tick();
    }));
    a.borrow_mut().text = "edited".into();
    registration.changed();
    a.borrow_mut().during_capture = None;
    assert_eq!(rig.session.save(), Some(json!({"a": "restored"})));
    rig.ui.tick();
    assert_eq!(texts(&a).0, "restored");
    assert_eq!(rig.session.save(), Some(json!({"a": "restored"})));
}

#[test]
fn a_restore_landing_during_an_apply_keeps_its_section_and_applies_after() {
    // The older apply's commit must not clear the newer restore.
    let rig = rig();
    let a = notes("x");
    let _a = rig.ui.register("a", &a).expect("free");
    let session = rig.session.clone();
    let fired = Rc::new(Cell::new(false));
    let once = Rc::clone(&fired);
    a.borrow_mut().during_apply = Some(Box::new(move || {
        if !once.replace(true) {
            session.restore(Some(&json!({"a": "newer"})));
        }
    }));
    rig.session.restore(Some(&json!({"a": "older"})));
    rig.ui.tick();
    assert_eq!(texts(&a).0, "older");
    assert_eq!(
        rig.session.save(),
        Some(json!({"a": "newer"})),
        "the newer raw section"
    );
    rig.ui.tick();
    assert_eq!(texts(&a).0, "newer", "its own pass");
    assert_eq!(rig.session.save(), Some(json!({"a": "newer"})));
    assert_eq!(rig.clock.stamp().completed(), 2, "two passes");
}

#[test]
fn a_participant_ending_its_registration_in_its_own_apply_is_not_recaptured() {
    let rig = rig();
    let a = notes("x");
    let registration = Rc::new(RefCell::new(Some(rig.ui.register("a", &a).expect("free"))));
    let own = Rc::clone(&registration);
    a.borrow_mut().during_apply = Some(Box::new(move || drop(own.borrow_mut().take())));
    rig.session.restore(Some(&json!({"a": "restored"})));
    rig.ui.tick();
    assert_eq!(texts(&a).0, "restored");
    assert_eq!(a.borrow().captures.get(), 1, "only at registration");
    assert!(registration.borrow().is_none());
}

#[test]
fn passes_never_nest() {
    // A tick inside an apply does nothing; a restore stored during a pass
    // gets its own later pass.
    let rig = rig();
    let a = notes("x");
    let b = notes("y");
    let _a = rig.ui.register("a", &a).expect("free");
    let _b = rig.ui.register("b", &b).expect("free");
    let ui = rig.ui.clone();
    let session = rig.session.clone();
    let fired = Rc::new(Cell::new(false));
    let once = Rc::clone(&fired);
    a.borrow_mut().during_apply = Some(Box::new(move || {
        if !once.replace(true) {
            session.restore(Some(&json!({"a": "second", "b": "second"})));
            ui.tick();
        }
    }));
    rig.session
        .restore(Some(&json!({"a": "first", "b": "first"})));
    rig.ui.tick();
    assert_eq!(texts(&a).1, [Some(json!("first"))]);
    assert_eq!(
        texts(&b).1,
        [Some(json!("second"))],
        "b's pending section was already the newest when the pass reached it"
    );
    rig.ui.tick();
    assert_eq!(texts(&a).1, [Some(json!("first")), Some(json!("second"))]);
    assert_eq!(texts(&b).1.len(), 1);
    assert_eq!(
        rig.clock.stamp().completed(),
        2,
        "no pass ran inside another"
    );
}

#[test]
fn the_session_writes_the_restore_clock() {
    // Stored as a restore stores, the applying mark around each apply,
    // completed once per pass, also with nobody registered.
    let rig = rig();
    rig.session.restore(None);
    assert_eq!(rig.clock.stamp().stored(), 1);
    assert_eq!(rig.clock.stamp().completed(), 0);
    rig.ui.tick();
    assert_eq!(
        rig.clock.stamp().completed(),
        1,
        "a pass with no participant"
    );

    let a = notes("x");
    a.borrow_mut().saw_applying = Some(rig.clock.clone());
    let _a = rig.ui.register("a", &a).expect("free");
    assert_eq!(
        a.borrow().applying_seen,
        [true],
        "the restore lacked the key: a late reset, under the mark"
    );
    assert_eq!(
        rig.clock.stamp().completed(),
        1,
        "a late apply completes nothing"
    );
    rig.session.restore(Some(&json!({"a": "y"})));
    rig.session.restore(Some(&json!({"a": "z"})));
    rig.ui.tick();
    assert_eq!(a.borrow().applying_seen, [true, true]);
    assert!(!rig.clock.applying());
    assert_eq!(
        (rig.clock.stamp().stored(), rig.clock.stamp().completed()),
        (3, 2),
        "two restores collapsed into one pass"
    );
    rig.ui.tick();
    assert_eq!(
        rig.clock.stamp().completed(),
        2,
        "a tick with nothing owed runs no pass"
    );
}

#[test]
fn a_participant_registering_after_a_restore_applies_its_section_then_captures_it() {
    // Before registration returns, restore-neutral, completing nothing; a
    // pass from the same restore does not apply it again.
    let rig = rig();
    rig.session.restore(Some(&json!({"late": "restored"})));
    let stamp = rig.clock.stamp();
    let late = notes("default");
    late.borrow_mut().saw_applying = Some(rig.clock.clone());
    let registration = rig.ui.register("late", &late).expect("free");
    assert_eq!(
        texts(&late),
        ("restored".into(), vec![Some(json!("restored"))])
    );
    assert_eq!(late.borrow().applying_seen, [true]);
    assert_eq!(
        late.borrow().captures.get(),
        2,
        "at registration and after the apply"
    );
    assert_eq!(rig.clock.stamp(), stamp, "neither count moved");
    rig.ui.tick();
    assert_eq!(
        late.borrow().applied.len(),
        1,
        "the owed pass leaves it alone"
    );
    late.borrow_mut().text = "edited".into();
    registration.changed();
    assert_eq!(rig.session.save(), Some(json!({"late": "edited"})));
}

#[test]
fn a_failed_late_apply_keeps_the_raw_section_and_the_registration_stands() {
    let rig = rig();
    rig.session.restore(Some(&json!({"late": {"v": 9}})));
    let late = notes("mine");
    late.borrow_mut().fail = true;
    let registration = rig.ui.register("late", &late).expect("free");
    assert!(registration.connected());
    assert_eq!(texts(&late).0, "mine");
    assert_eq!(rig.session.save(), Some(json!({"late": {"v": 9}})));
}

#[test]
fn a_late_participant_the_newest_restore_lacked_is_reset_once() {
    // A restore is a complete assignment once per key and restore;
    // with no restore yet, registering applies nothing.
    let rig = rig();
    let early = notes("mine");
    let registration = rig.ui.register("k", &early).expect("free");
    assert!(early.borrow().applied.is_empty(), "no restore yet");
    drop(registration);

    rig.session.restore(Some(&json!({"other": 1})));
    let first = notes("");
    let registration = rig.ui.register("k", &first).expect("free");
    assert_eq!(texts(&first), ("default".into(), vec![None]));
    first.borrow_mut().text = String::new();
    registration.changed();
    drop(registration);
    let again = notes("again");
    let registration = rig.ui.register("k", &again).expect("free");
    assert!(
        again.borrow().applied.is_empty(),
        "already reset for this restore"
    );
    again.borrow_mut().text = String::new();
    registration.changed();
    drop(registration);
    rig.session.restore(Some(&json!({"other": 2})));
    let _again = rig.ui.register("k", &again).expect("free");
    assert_eq!(texts(&again).1, [None], "reset again for the next restore");

    // Held through a restore that lacks it, the pass resets it; ending and
    // re-registering does not reset it twice.
    drop(_again);
    let held = notes("held");
    let registration = rig.ui.register("held", &held).expect("free");
    rig.session.restore(Some(&json!({})));
    rig.ui.tick();
    assert_eq!(
        texts(&held).1,
        [None, None],
        "late for the first restore, then the second's pass"
    );
    held.borrow_mut().text = String::new();
    registration.changed();
    drop(registration);
    let back = notes("back");
    let _back = rig.ui.register("held", &back).expect("free");
    assert!(back.borrow().applied.is_empty());
}

#[test]
fn ending_a_registration_keeps_its_section_for_every_save_and_the_next_registration() {
    // An editor's layout survives the editor closing; the next one takes it
    // over whatever it holds.
    let rig = rig();
    let editor = notes("layout");
    let registration = rig.ui.register("editor", &editor).expect("free");
    drop(registration);
    assert_eq!(rig.session.save(), Some(json!({"editor": "layout"})));
    let reopened = notes("fresh");
    let _reopened = rig.ui.register("editor", &reopened).expect("free");
    assert_eq!(
        texts(&reopened),
        ("layout".into(), vec![Some(json!("layout"))])
    );
    assert_eq!(
        rig.session.save(),
        Some(json!({"editor": "layout"})),
        "held, not kept twice"
    );
}

#[test]
fn a_restore_while_a_key_is_unheld_replaces_or_drops_its_kept_section() {
    for carries in [true, false] {
        let rig = rig();
        let editor = notes("layout");
        drop(rig.ui.register("editor", &editor).expect("free"));
        let extra = if carries {
            json!({"editor": "newer"})
        } else {
            json!({})
        };
        rig.session.restore(Some(&extra));
        assert_eq!(
            rig.session.save(),
            (!extra.as_object().expect("an object").is_empty()).then_some(extra.clone())
        );
        let reopened = notes("fresh");
        let _reopened = rig.ui.register("editor", &reopened).expect("free");
        let expected = if carries { Some(json!("newer")) } else { None };
        assert_eq!(texts(&reopened).1, [expected], "carries: {carries}");
    }
}

#[test]
fn a_last_capture_of_nothing_leaves_nothing_behind() {
    // How to forget a section.
    let rig = rig();
    let editor = notes("layout");
    let registration = rig.ui.register("editor", &editor).expect("free");
    editor.borrow_mut().text = String::new();
    registration.changed();
    drop(registration);
    assert_eq!(rig.session.save(), None);
    let reopened = notes("fresh");
    let _reopened = rig.ui.register("editor", &reopened).expect("free");
    assert!(reopened.borrow().applied.is_empty());
}

#[test]
fn a_registration_ending_before_its_pending_apply_keeps_the_restored_section() {
    // The raw restored section stays, unapplied, for the next one.
    let rig = rig();
    let editor = notes("old");
    let registration = rig.ui.register("editor", &editor).expect("free");
    rig.session.restore(Some(&json!({"editor": "restored"})));
    drop(registration);
    rig.ui.tick();
    assert_eq!(texts(&editor).0, "old");
    assert_eq!(rig.session.save(), Some(json!({"editor": "restored"})));
    let reopened = notes("fresh");
    let _reopened = rig.ui.register("editor", &reopened).expect("free");
    assert_eq!(texts(&reopened).0, "restored");
}

#[test]
fn a_participant_registering_while_a_pass_is_owed_applies_once_at_registration() {
    // The pass that follows leaves it alone, unless a newer restore stored
    // its section first.
    for newer in [false, true] {
        let rig = rig();
        let other = notes("o");
        let _other = rig.ui.register("other", &other).expect("free");
        rig.session
            .restore(Some(&json!({"late": "first", "other": "x"})));
        let late = notes("default");
        let _late = rig.ui.register("late", &late).expect("free");
        if newer {
            rig.session
                .restore(Some(&json!({"late": "second", "other": "y"})));
        }
        rig.ui.tick();
        let expected = if newer {
            vec![Some(json!("first")), Some(json!("second"))]
        } else {
            vec![Some(json!("first"))]
        };
        assert_eq!(texts(&late).1, expected, "newer: {newer}");
    }
}

#[test]
fn a_participant_registering_inside_another_apply_applies_once_with_the_newest_section() {
    // The pass's list is the one it began with.
    let rig = rig();
    let host = notes("h");
    let _host = rig.ui.register("host", &host).expect("free");
    let ui = rig.ui.clone();
    let session = rig.session.clone();
    let inner = notes("default");
    let registered = Rc::new(RefCell::new(None));
    let (slot, participant) = (Rc::clone(&registered), Rc::clone(&inner));
    host.borrow_mut().during_apply = Some(Box::new(move || {
        if slot.borrow().is_none() {
            session.restore(Some(&json!({"host": "H2", "inner": "I2"})));
            *slot.borrow_mut() = Some(ui.register("inner", &participant).expect("free"));
        }
    }));
    rig.session
        .restore(Some(&json!({"host": "H", "inner": "I"})));
    rig.ui.tick();
    assert_eq!(texts(&inner).1, [Some(json!("I2"))], "the newest, once");
    rig.ui.tick();
    assert_eq!(texts(&inner).1.len(), 1);
    assert_eq!(texts(&host).0, "H2", "the host's own pass");
    assert!(registered.borrow().is_some());
}

#[test]
fn a_section_is_claimed_by_its_exact_key_only() {
    let rig = rig();
    rig.session
        .restore(Some(&json!({"Editor": "a", "editor2": "b"})));
    let editor = notes("mine");
    let _editor = rig.ui.register("editor", &editor).expect("free");
    assert_eq!(texts(&editor).1, [None], "nothing under its key: the reset");
    assert_eq!(
        rig.session.save(),
        Some(json!({"Editor": "a", "editor": "default", "editor2": "b"}))
    );
}

/// A participant that tells the session from its own setter, as an editor
/// does from an event handler.
struct Zoom {
    level: u32,
    registration: Option<seam_session::Registration>,
}

impl Zoom {
    fn set(&mut self, level: u32) {
        self.level = level;
        if let Some(registration) = &self.registration {
            registration.changed();
        }
    }
}

impl Participant for Zoom {
    fn capture(&self) -> Section {
        json!(self.level)
    }

    fn apply(&mut self, section: Option<&Section>) -> Result<(), ApplyError> {
        self.level = section.and_then(Section::as_u64).unwrap_or(1) as u32;
        Ok(())
    }
}

#[test]
fn a_change_told_from_the_participants_own_method_saves_after_the_next_tick() {
    // The setter runs under the owner's borrow, so the capture waits for
    // the tick rather than being lost.
    let rig = rig();
    let zoom = Rc::new(RefCell::new(Zoom {
        level: 1,
        registration: None,
    }));
    let registration = rig.ui.register("zoom", &zoom).expect("free");
    zoom.borrow_mut().registration = Some(registration);
    zoom.borrow_mut().set(4);
    assert_eq!(rig.session.save(), Some(json!({"zoom": 1})), "not yet");
    rig.ui.tick();
    assert_eq!(rig.session.save(), Some(json!({"zoom": 4})));
    zoom.borrow_mut().set(5);
    rig.session.restore(Some(&json!({"zoom": 9})));
    rig.ui.tick();
    assert_eq!(
        zoom.borrow().level,
        9,
        "the restore outranks the pending change"
    );
    assert_eq!(rig.session.save(), Some(json!({"zoom": 9})));
}

#[test]
fn dropping_the_session_ui_ends_its_registrations_and_keeps_their_sections() {
    let rig = rig();
    let editor = notes("layout");
    let registration = rig.ui.register("editor", &editor).expect("free");
    let Rig { session, ui, .. } = rig;
    drop(ui);
    assert!(!registration.connected());
    assert_eq!(session.save(), Some(json!({"editor": "layout"})));
    let ui = session.take_ui().expect("a new handle");
    let reopened = notes("fresh");
    let _reopened = ui.register("editor", &reopened).expect("the key is free");
    assert_eq!(texts(&reopened).0, "layout");
    drop(registration);
    assert_eq!(session.save(), Some(json!({"editor": "layout"})));
}

thread_local! {
    static OWNER: RefCell<Option<Rc<RefCell<Fragile>>>> = const { RefCell::new(None) };
}

/// A participant that drops its owner's last handle inside its apply and
/// panics as it drops.
struct Fragile;

impl Participant for Fragile {
    fn capture(&self) -> Section {
        Section::Null
    }

    fn apply(&mut self, _: Option<&Section>) -> Result<(), ApplyError> {
        OWNER.with(|owner| drop(owner.borrow_mut().take()));
        Ok(())
    }
}

impl Drop for Fragile {
    fn drop(&mut self) {
        panic!("a participant's drop panicked");
    }
}

#[test]
fn a_pass_a_panic_escapes_is_owed_again_and_later_restores_still_apply() {
    // A stuck pass would stop every later restore.
    let rig = rig();
    let fragile = Rc::new(RefCell::new(Fragile));
    let _fragile = rig.ui.register("fragile", &fragile).expect("free");
    OWNER.with(|owner| *owner.borrow_mut() = Some(fragile));
    let other = notes("o");
    let _other = rig.ui.register("other", &other).expect("free");
    rig.session
        .restore(Some(&json!({"fragile": 1, "other": "first"})));
    let escaped = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| rig.ui.tick()));
    assert!(escaped.is_err());
    rig.ui.tick();
    assert_eq!(texts(&other).0, "first", "the pass finished");
    assert_eq!(rig.clock.stamp().completed(), 1);
    rig.session.restore(Some(&json!({"other": "second"})));
    rig.ui.tick();
    assert_eq!(texts(&other).0, "second");
}

#[test]
fn a_restore_landing_during_a_late_apply_keeps_its_section_and_applies_after() {
    // The older apply may itself be a late one.
    let rig = rig();
    rig.session.restore(Some(&json!({"late": "older"})));
    let late = notes("default");
    let session = rig.session.clone();
    let fired = Rc::new(Cell::new(false));
    let once = Rc::clone(&fired);
    late.borrow_mut().during_apply = Some(Box::new(move || {
        if !once.replace(true) {
            session.restore(Some(&json!({"late": "newer"})));
        }
    }));
    let _late = rig.ui.register("late", &late).expect("free");
    assert_eq!(texts(&late).0, "older");
    assert_eq!(rig.session.save(), Some(json!({"late": "newer"})));
    rig.ui.tick();
    assert_eq!(texts(&late).0, "newer");
    assert_eq!(rig.session.save(), Some(json!({"late": "newer"})));
}

#[test]
fn a_tick_inside_a_late_apply_does_nothing() {
    // A late apply is not a pass, and runs none.
    let rig = rig();
    let other = notes("o");
    let _other = rig.ui.register("other", &other).expect("free");
    rig.session
        .restore(Some(&json!({"late": "x", "other": "y"})));
    let late = notes("default");
    let ui = rig.ui.clone();
    late.borrow_mut().during_apply = Some(Box::new(move || ui.tick()));
    let _late = rig.ui.register("late", &late).expect("free");
    assert!(other.borrow().applied.is_empty(), "no pass ran inside it");
    assert_eq!(rig.clock.stamp().completed(), 0);
    rig.ui.tick();
    assert_eq!(texts(&other).0, "y");
}

#[test]
fn a_panicking_capture_or_a_busy_participant_registers_nothing() {
    // The capture comes first, so a waiting section stays where it was.
    let rig = rig();
    rig.session.restore(Some(&json!({"k": "waiting"})));
    let panics = notes("x");
    panics.borrow_mut().during_capture = Some(Box::new(|| panic!("a capture panicked")));
    let escaped = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        drop(rig.ui.register("k", &panics));
    }));
    assert!(escaped.is_err());
    assert_eq!(rig.session.save(), Some(json!({"k": "waiting"})));
    let busy = notes("y");
    let held = busy.borrow_mut();
    assert_eq!(
        rig.ui.register("k", &busy).map(|_| ()),
        Err(SessionError::Busy)
    );
    drop(held);
    let _busy = rig.ui.register("k", &busy).expect("free");
    assert_eq!(texts(&busy).0, "waiting");
}

#[test]
fn a_panicking_capture_reaches_the_caller_of_changed_but_a_tick_contains_it() {
    // A tick has no caller to reach, so it goes on to the pass owed.
    let rig = rig();
    rig.session.restore(None);
    let a = notes("x");
    let registration = rig.ui.register("a", &a).expect("free");
    a.borrow_mut().during_capture = Some(Box::new(|| panic!("a capture panicked")));
    let escaped = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| registration.changed()));
    assert!(escaped.is_err());
    let held = a.borrow_mut();
    registration.changed();
    drop(held);
    rig.ui.tick();
    assert_eq!(
        rig.session.save(),
        Some(json!({"a": "default"})),
        "the section saved before stays"
    );
    assert_eq!(rig.clock.stamp().completed(), 1, "the owed pass ran");
}

#[test]
fn a_tick_inside_a_held_up_capture_captures_the_others_still_held_up() {
    // So a held-up change survives the pass that inner tick runs, here one
    // that ends the held-up registration.
    let rig = rig();
    let (a, b, c) = (notes("a"), notes("b"), notes("c"));
    let registration_a = rig.ui.register("a", &a).expect("free");
    let registration_b = Rc::new(RefCell::new(Some(rig.ui.register("b", &b).expect("free"))));
    let _c = rig.ui.register("c", &c).expect("free");
    rig.session
        .restore(Some(&json!({"a": "a1", "b": "b1", "c": "c1"})));
    let busy = c.borrow_mut();
    rig.ui.tick();
    drop(busy);
    let held = a.borrow_mut();
    registration_a.changed();
    drop(held);
    let held = b.borrow_mut();
    registration_b
        .borrow()
        .as_ref()
        .expect("registered")
        .changed();
    drop(held);
    b.borrow_mut().text = "b2".into();
    let ending = Rc::clone(&registration_b);
    c.borrow_mut().during_apply = Some(Box::new(move || drop(ending.borrow_mut().take())));
    let ui = rig.ui.clone();
    a.borrow_mut().during_capture = Some(Box::new(move || ui.tick()));
    rig.ui.tick();
    assert_eq!(texts(&c).0, "c1", "the owed pass ran inside a's capture");
    assert!(registration_b.borrow().is_none());
    assert_eq!(
        rig.session.save(),
        Some(json!({"a": "a1", "b": "b2", "c": "c1"})),
        "b's held-up change, left under its key"
    );
}

/// Runs its closure when dropped.
struct OnDrop<F: FnMut()>(F);

impl<F: FnMut()> Drop for OnDrop<F> {
    fn drop(&mut self) {
        (self.0)();
    }
}

#[test]
fn a_registration_made_while_unwinding_owes_no_pass() {
    // Only a panic that began inside a pass or a late apply leaves it
    // unfinished, and a pass owed for nothing would clear every undo history.
    let rig = rig();
    let a = notes("x");
    let registration = RefCell::new(None);
    let unwound = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _register = OnDrop(|| {
            *registration.borrow_mut() = Some(rig.ui.register("a", &a).expect("free"));
        });
        panic!("unwinding");
    }));
    assert!(unwound.is_err());
    assert!(registration.borrow().is_some());
    rig.ui.tick();
    assert_eq!(rig.clock.stamp().completed(), 0, "no pass was owed");
}

#[test]
fn two_host_threads_restoring_at_once_settle_on_one_applied_once() {
    // Whichever stores last wins, and the pass applies it once.
    for _ in 0..50 {
        let rig = rig();
        let a = notes("a");
        let _a = rig.ui.register("a", &a).expect("free");
        let threads: Vec<_> = ["one", "two"]
            .into_iter()
            .map(|text| {
                let session = rig.session.clone();
                std::thread::spawn(move || session.restore(Some(&json!({"a": text}))))
            })
            .collect();
        for thread in threads {
            thread.join().expect("a restoring thread");
        }
        let saved = rig.session.save().expect("a section");
        rig.ui.tick();
        let (text, applied) = texts(&a);
        assert_eq!(applied.len(), 1, "applied once");
        assert_eq!(
            Some(json!(text)),
            saved.get("a").cloned(),
            "saved as applied"
        );
        assert_eq!(rig.session.save(), Some(saved));
    }
}

#[test]
fn a_pass_that_left_a_busy_participant_owed_is_not_completed() {
    // Marking it completed would clear every undo history at every tick
    // for as long as the participant stays busy.
    let rig = rig();
    let a = notes("x");
    let _a = rig.ui.register("a", &a).expect("free");
    rig.session.restore(Some(&json!({"a": "restored"})));
    let held = a.borrow_mut();
    for _ in 0..3 {
        rig.ui.tick();
    }
    assert_eq!(
        rig.clock.stamp().completed(),
        0,
        "unfinished, not completed"
    );
    drop(held);
    rig.ui.tick();
    assert_eq!(texts(&a).0, "restored");
    assert_eq!(rig.clock.stamp().completed(), 1);
}

#[test]
fn a_finished_pass_covers_the_stored_count_it_began_at() {
    // What the saved state reads to tell a pass that applied a restore from
    // one that began before the restore stored its sections.
    let rig = rig();
    assert_eq!(rig.ui.covered(), 0, "before any pass");
    let a = notes("x");
    let _a = rig.ui.register("a", &a).expect("free");

    rig.session.restore(Some(&json!({"a": "one"})));
    let stored = rig.clock.stamp().stored();
    assert_eq!(rig.ui.covered(), 0, "stored, not applied");
    rig.ui.tick();
    assert_eq!(rig.ui.covered(), stored);

    // A participant busy through the pass leaves it owed and unfinished.
    rig.session.restore(Some(&json!({"a": "two"})));
    let busy = a.borrow();
    rig.ui.tick();
    assert_eq!(
        rig.ui.covered(),
        stored,
        "an unfinished pass covers nothing"
    );
    drop(busy);
    rig.ui.tick();
    assert_eq!(rig.ui.covered(), rig.clock.stamp().stored());

    // A restore stored inside a pass's apply is past what that pass covers:
    // the next pass covers it.
    let session = rig.session.clone();
    let fired = Rc::new(Cell::new(false));
    let once = Rc::clone(&fired);
    a.borrow_mut().during_apply = Some(Box::new(move || {
        if !once.replace(true) {
            session.restore(Some(&json!({"a": "four"})));
        }
    }));
    rig.session.restore(Some(&json!({"a": "three"})));
    let began = rig.clock.stamp().stored();
    rig.ui.tick();
    assert_eq!(rig.ui.covered(), began);
    assert!(rig.clock.stamp().stored() > began, "the restore inside it");
    rig.ui.tick();
    assert_eq!(rig.ui.covered(), rig.clock.stamp().stored());
    assert_eq!(texts(&a).0, "four");
}
