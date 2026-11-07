//! A capture that would grow the `extra` object past the budget the saved
//! state sets is refused, as one nested too deep is, so a save never writes a
//! block longer than a load reads.

use std::cell::RefCell;
use std::rc::Rc;

use seam_params::ParamSetBuilder;
use seam_session::{
    ApplyError, MAX_STATE_BYTES, Participant, Registration, Section, Session, SessionUi,
};
use serde_json::json;

/// A participant whose section a test sets directly.
#[derive(Debug, PartialEq)]
struct Fixed(Section);

impl Participant for Fixed {
    fn capture(&self) -> Section {
        self.0.clone()
    }

    fn apply(&mut self, section: Option<&Section>) -> Result<(), ApplyError> {
        self.0 = section.cloned().unwrap_or(Section::Null);
        Ok(())
    }
}

fn session() -> (Session, SessionUi) {
    let session = Session::new(&ParamSetBuilder::new().build());
    let ui = session.take_ui().expect("the first UI handle");
    (session, ui)
}

/// The bytes of the `extra` object a save writes, as its text.
fn extra_len(session: &Session) -> usize {
    session.save().map_or(0, |extra| extra.to_string().len())
}

struct Held {
    participant: Rc<RefCell<Fixed>>,
    registration: Registration,
}

impl Held {
    fn new(ui: &SessionUi, key: &str, section: Section) -> Held {
        let participant = Rc::new(RefCell::new(Fixed(section)));
        let registration = ui.register(key, &participant).expect("free");
        Held {
            participant,
            registration,
        }
    }

    fn capture(&self, section: Section) {
        self.participant.borrow_mut().0 = section;
        self.registration.changed();
    }
}

/// A text of `count` characters that each take two bytes or more in JSON: a
/// quote, a backslash, a control character and a two-byte letter in turn.
fn escaped(count: usize) -> String {
    ['"', '\\', '\u{1}', '\u{e9}']
        .iter()
        .cycle()
        .take(count)
        .collect()
}

#[test]
fn the_largest_capture_that_fits_fills_the_budget_exactly_and_one_byte_more_is_refused() {
    let (session, ui) = session();
    let zoom = Held::new(&ui, "zoom", json!("x"));
    // `{"zoom":"` and `"}`, around the text.
    let budget = 11 + 40;
    session.set_budget(budget);

    zoom.capture(json!("x".repeat(40)));
    assert!(!zoom.registration.refused());
    assert_eq!(extra_len(&session), budget, "exactly the budget");

    zoom.capture(json!("x".repeat(41)));
    assert!(zoom.registration.refused(), "one byte past it");
    assert_eq!(session.save(), Some(json!({ "zoom": "x".repeat(40) })));

    zoom.capture(json!("short"));
    assert!(!zoom.registration.refused(), "the next capture that fits");
    assert_eq!(session.save(), Some(json!({ "zoom": "short" })));
}

#[test]
fn escapes_in_keys_and_sections_are_counted_as_a_save_writes_them() {
    let (session, ui) = session();
    let key = escaped(5);
    let nested = Held::new(&ui, &key, Section::Null);
    let text = Held::new(&ui, "text", Section::Null);
    let full = json!({ key.as_str(): [escaped(30), {"a": escaped(7)}], "text": escaped(9) });
    let budget = full.to_string().len();
    session.set_budget(budget);

    nested.capture(full[key.as_str()].clone());
    text.capture(full["text"].clone());
    assert!(!nested.registration.refused() && !text.registration.refused());
    assert_eq!(extra_len(&session), budget);

    text.capture(json!(escaped(10)));
    assert!(text.registration.refused());
    nested.capture(json!([escaped(30), {"a": escaped(8)}]));
    assert!(nested.registration.refused());
    assert_eq!(session.save(), Some(full));
}

#[test]
fn every_section_shares_the_budget_those_no_participant_holds_included() {
    let (session, ui) = session();
    let a = Held::new(&ui, "a", json!("a".repeat(10)));
    let b = Held::new(&ui, "b", json!("b".repeat(10)));
    // An ended registration leaves its section to the next one of its key.
    drop(Held::new(&ui, "ended", json!("e".repeat(20))));
    assert_eq!(
        session.save().map(|extra| extra["ended"].clone()),
        Some(json!("e".repeat(20)))
    );
    let budget = extra_len(&session);
    session.set_budget(budget);

    a.capture(json!("a".repeat(11)));
    assert!(a.registration.refused(), "no room left");
    b.capture(json!("b".repeat(5)));
    assert!(!b.registration.refused());
    a.capture(json!("a".repeat(15)));
    assert!(!a.registration.refused(), "the room b gave up");
    assert_eq!(extra_len(&session), budget);
}

#[test]
fn a_registration_whose_capture_has_no_room_saves_nothing_under_its_key() {
    let (session, ui) = session();
    let _small = Held::new(&ui, "small", json!(1));
    // Room for `,"large":2` and no more.
    session.set_budget(extra_len(&session) + 10);
    let large = Held::new(&ui, "large", json!("l".repeat(100)));
    assert!(large.registration.refused());
    assert_eq!(session.save(), Some(json!({ "small": 1 })));

    large.capture(json!(2));
    assert!(!large.registration.refused());
    assert_eq!(session.save(), Some(json!({ "large": 2, "small": 1 })));
    large.capture(json!(20));
    assert!(large.registration.refused());
}

#[test]
fn a_restore_is_never_refused_and_a_capture_no_larger_than_what_it_replaces_is_taken() {
    let (session, ui) = session();
    let zoom = Held::new(&ui, "zoom", json!("x"));
    session.set_budget(extra_len(&session));
    let restored = json!({ "zoom": "r".repeat(50) });
    session.restore(Some(&restored));
    assert_eq!(session.save(), Some(restored.clone()), "past the budget");
    ui.tick();
    assert!(!zoom.registration.refused(), "its re-capture is no larger");

    zoom.capture(json!("r".repeat(51)));
    assert!(zoom.registration.refused(), "growing past the budget");
    assert_eq!(session.save(), Some(restored));
    zoom.capture(json!("r".repeat(30)));
    assert!(
        !zoom.registration.refused(),
        "shrinking, though still past it"
    );
    assert_eq!(session.save(), Some(json!({ "zoom": "r".repeat(30) })));
}

/// A participant whose apply makes its state far larger than its section.
struct Swells(Section);

impl Participant for Swells {
    fn capture(&self) -> Section {
        self.0.clone()
    }

    fn apply(&mut self, _: Option<&Section>) -> Result<(), ApplyError> {
        self.0 = json!("s".repeat(1_000));
        Ok(())
    }
}

#[test]
fn a_recapture_after_an_apply_that_has_no_room_keeps_the_restored_section() {
    let (session, ui) = session();
    let participant = Rc::new(RefCell::new(Swells(json!(0))));
    let registration = ui.register("swells", &participant).expect("free");
    session.set_budget(100);
    session.restore(Some(&json!({ "swells": [1, 2] })));
    ui.tick();
    assert!(registration.refused());
    assert_eq!(session.save(), Some(json!({ "swells": [1, 2] })));
}

#[test]
fn a_session_no_saved_state_set_a_budget_for_takes_sections_up_to_the_whole_block() {
    let (session, ui) = session();
    let large = Held::new(&ui, "large", json!("l".repeat(1 << 20)));
    assert!(!large.registration.refused());
    assert_eq!(extra_len(&session), 12 + (1 << 20));

    // `{"large":"` and `"}` around the text.
    large.capture(json!("l".repeat(MAX_STATE_BYTES - 12)));
    assert!(!large.registration.refused(), "the whole block");
    assert_eq!(extra_len(&session), MAX_STATE_BYTES);
    large.capture(json!("l".repeat(MAX_STATE_BYTES - 11)));
    assert!(large.registration.refused(), "a byte past it");
}

#[test]
fn a_lowered_budget_drops_the_sections_past_it_in_registration_order_and_refuses_them() {
    let (session, ui) = session();
    let first = Held::new(&ui, "first", json!("f".repeat(10)));
    let large = Held::new(&ui, "large", json!("l".repeat(100)));
    let last = Held::new(&ui, "last", json!("t".repeat(10)));
    // An ended registration leaves its section to the next one of its key.
    drop(Held::new(&ui, "ended", json!("e".repeat(10))));
    let all = session.save();

    // Raised or kept, the budget drops nothing.
    session.set_budget(extra_len(&session));
    assert_eq!(session.save(), all);
    assert!(!first.registration.refused() && !large.registration.refused());

    // Room for every section but `large`: those after it still fit.
    let without =
        json!({ "ended": "e".repeat(10), "first": "f".repeat(10), "last": "t".repeat(10) });
    session.set_budget(without.to_string().len());
    assert_eq!(session.save(), Some(without));
    assert!(large.registration.refused(), "its key saves nothing");
    assert!(!first.registration.refused() && !last.registration.refused());

    // Room for `first` alone: the later ones, `ended` last, are dropped.
    session.set_budget(json!({ "first": "f".repeat(10) }).to_string().len());
    assert_eq!(session.save(), Some(json!({ "first": "f".repeat(10) })));
    assert!(last.registration.refused());

    // A capture that fits clears the mark.
    session.set_budget(MAX_STATE_BYTES);
    large.capture(json!("small"));
    assert!(!large.registration.refused());
    assert_eq!(
        session.save(),
        Some(json!({ "first": "f".repeat(10), "large": "small" }))
    );
}

#[test]
fn a_lowered_budget_drops_a_restored_section_waiting_for_its_apply_and_the_apply_with_it() {
    let (session, ui) = session();
    let held = Held::new(&ui, "held", json!(1));
    session.restore(Some(&json!({ "held": "r".repeat(100) })));
    session.set_budget(20);
    assert!(held.registration.refused());
    assert_eq!(session.save(), None);

    ui.tick();
    assert_eq!(session.save(), None, "the apply saves nothing back");
    assert_eq!(
        *held.participant.borrow(),
        Fixed(json!(1)),
        "nor applies it"
    );
    held.registration.changed();
    assert!(!held.registration.refused());
    assert_eq!(session.save(), Some(json!({ "held": 1 })));
}

#[test]
fn sections_no_participant_holds_are_judged_in_key_order_whatever_order_they_came_in() {
    let (session, ui) = session();
    // Ended in this order, `b` then `a`, which is how they are kept.
    drop(Held::new(&ui, "b", json!("b".repeat(10))));
    drop(Held::new(&ui, "a", json!("a".repeat(10))));
    let one = json!({ "a": "a".repeat(10) });
    session.set_budget(one.to_string().len());
    assert_eq!(session.save(), Some(one));
}

#[test]
fn a_restored_section_a_lowered_budget_drops_is_not_applied_to_a_later_registration() {
    let (session, ui) = session();
    session.restore(Some(&json!({ "later": "r".repeat(100) })));
    session.set_budget(20);
    assert_eq!(session.save(), None, "dropped, though no one holds it");

    let later = Held::new(&ui, "later", json!(1));
    assert!(!later.registration.refused());
    assert_eq!(*later.participant.borrow(), Fixed(json!(1)));
    assert_eq!(session.save(), Some(json!({ "later": 1 })));
}
