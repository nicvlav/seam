//! A section deeper than the saved block can be read back is refused at
//! capture, so what a save writes always loads.

use std::cell::RefCell;
use std::rc::Rc;

use seam_params::ParamSetBuilder;
use seam_session::{ApplyError, MAX_SECTION_DEPTH, Participant, Section, Session};
use serde_json::{Value, json};

/// A section of `levels` nested arrays around one number: `levels` deep.
fn nested(levels: usize) -> Section {
    let mut node = json!(1);
    for _ in 0..levels {
        node = Section::Array(vec![node]);
    }
    node
}

/// A participant whose section a test sets directly.
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

/// The block a host writes around a session's `extra`, as text.
fn block(extra: &Section) -> String {
    format!(r#"{{"seamStateVersion":1,"parameters":{{}},"extra":{extra}}}"#)
}

/// Returns whether `serde_json` reads the block back, as a host's load does.
fn loads(extra: &Section) -> bool {
    serde_json::from_slice::<Value>(block(extra).as_bytes()).is_ok()
}

#[test]
fn a_section_at_the_depth_limit_saves_a_block_that_loads() {
    let set = ParamSetBuilder::new().build();
    let session = Session::new(&set);
    let ui = session.take_ui().expect("the first UI handle");
    let participant = Rc::new(RefCell::new(Fixed(nested(MAX_SECTION_DEPTH))));
    let registration = ui.register("deep", &participant).expect("free");
    assert!(!registration.refused());
    let saved = session.save().expect("a section");
    assert!(loads(&saved), "{MAX_SECTION_DEPTH} levels must load");
    assert!(
        !loads(&json!({ "deep": nested(MAX_SECTION_DEPTH + 1) })),
        "the limit is exactly the deepest section that loads"
    );
}

#[test]
fn a_section_past_the_limit_is_refused_and_the_previous_one_stays_saved() {
    let set = ParamSetBuilder::new().build();
    let session = Session::new(&set);
    let ui = session.take_ui().expect("the first UI handle");
    let participant = Rc::new(RefCell::new(Fixed(json!("before"))));
    let registration = ui.register("deep", &participant).expect("free");

    participant.borrow_mut().0 = nested(MAX_SECTION_DEPTH + 1);
    registration.changed();
    assert!(registration.refused());
    assert_eq!(session.save(), Some(json!({ "deep": "before" })));
    ui.tick();
    assert_eq!(session.save(), Some(json!({ "deep": "before" })));

    participant.borrow_mut().0 = json!("after");
    registration.changed();
    assert!(!registration.refused());
    assert_eq!(session.save(), Some(json!({ "deep": "after" })));
}

#[test]
fn a_tree_of_sixty_two_levels_is_refused() {
    // Each level of the tree is an object holding an array: two JSON levels.
    fn tree(levels: usize) -> Section {
        let mut node = json!({"name": "leaf", "children": []});
        for level in 0..levels {
            node = json!({"name": format!("group {level}"), "children": [node]});
        }
        node
    }
    let set = ParamSetBuilder::new().build();
    let session = Session::new(&set);
    let ui = session.take_ui().expect("the first UI handle");
    let kept = Rc::new(RefCell::new(Fixed(tree(61))));
    let refused = Rc::new(RefCell::new(Fixed(tree(62))));
    let _kept = ui.register("kept", &kept).expect("free");
    let refused_registration = ui.register("refused", &refused).expect("free");
    assert!(refused_registration.refused());
    let saved = session.save().expect("one section");
    assert!(saved.get("refused").is_none(), "nothing was saved before");
    assert!(loads(&saved));
}

#[test]
fn a_restored_section_whose_recapture_is_too_deep_stays_as_restored() {
    /// Shallow until an apply, too deep after it.
    struct Grows(bool);
    impl Participant for Grows {
        fn capture(&self) -> Section {
            if self.0 {
                nested(MAX_SECTION_DEPTH + 1)
            } else {
                json!(0)
            }
        }
        fn apply(&mut self, _: Option<&Section>) -> Result<(), ApplyError> {
            self.0 = true;
            Ok(())
        }
    }
    let set = ParamSetBuilder::new().build();
    let session = Session::new(&set);
    let ui = session.take_ui().expect("the first UI handle");
    let participant = Rc::new(RefCell::new(Grows(false)));
    let registration = ui.register("grows", &participant).expect("free");
    assert!(!registration.refused());
    session.restore(Some(&json!({ "grows": [1, 2] })));
    ui.tick();
    assert!(registration.refused(), "the apply's re-capture");
    assert_eq!(session.save(), Some(json!({ "grows": [1, 2] })));
}

#[test]
fn a_refused_participant_held_by_its_own_code_is_captured_by_the_next_tick() {
    let set = ParamSetBuilder::new().build();
    let session = Session::new(&set);
    let ui = session.take_ui().expect("the first UI handle");
    let participant = Rc::new(RefCell::new(Fixed(json!("before"))));
    let registration = ui.register("deep", &participant).expect("free");

    participant.borrow_mut().0 = nested(MAX_SECTION_DEPTH + 1);
    registration.changed();
    assert!(registration.refused());

    // Its own code holds it: the change waits, and the refusal stays.
    let mut held = participant.borrow_mut();
    held.0 = json!("after");
    registration.changed();
    assert!(registration.refused());
    assert_eq!(session.save(), Some(json!({ "deep": "before" })));
    drop(held);

    ui.tick();
    assert!(!registration.refused());
    assert_eq!(session.save(), Some(json!({ "deep": "after" })));
}
