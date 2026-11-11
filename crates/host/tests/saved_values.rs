//! A plain value saved with the session through `Parts::save`: captured at the
//! tick, loaded back or defaulted, refused under a taken key, and captured
//! once for a load.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use seam_host::{
    AuCodes, Block, Category, Driver, Layout, Parts, Plugin, PluginInfo, ProcessSetup, Processor,
    Status,
};
use seam_params::{Declarer, Params, Value};
use seam_session::{ApplyError, Participant, Section, SessionError};
use serde::{Deserialize, Serialize, Serializer};
use serde_json::json;

#[derive(Debug, Clone)]
struct NoParams;

impl Params for NoParams {
    fn declare(_: &mut Declarer) -> Self {
        NoParams
    }
}

struct Silent;

impl Processor for Silent {
    fn process(&mut self, _: Block<'_, '_>) -> Status {
        Status::Sleep
    }
}

/// A plugin with nothing of its own: only its parts are used.
struct Bare;

impl Plugin for Bare {
    const INFO: PluginInfo = PluginInfo {
        name: "Bare",
        vendor: "Seam",
        url: "",
        email: "",
        description: "",
        version: "0.1.0",
        id: "dev.seam.bare",
        vst3_class_id: [0; 16],
        au_codes: AuCodes {
            subtype: *b"bare",
            manufacturer: *b"Seam",
        },
        category: Category::Effect,
        features: &[],
        layouts: &[Layout::STEREO],
        midi_input: false,
        midi_output: false,
    };
    type Params = NoParams;
    type Processor = Silent;

    fn new(_: NoParams, _: Parts) -> Self {
        Bare
    }

    fn activate(&mut self, _: &ProcessSetup) -> Silent {
        Silent
    }
}

/// An editor's view, saved as one value.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct View {
    zoom: f64,
    panel: String,
}

fn parts() -> Parts {
    Parts::for_plugin::<Bare>()
        .expect("no declarations to refuse")
        .1
}

/// The `extra` object the host's next save holds.
fn extra(parts: &Parts) -> serde_json::Value {
    let block: serde_json::Value =
        serde_json::from_slice(&parts.saved().save()).expect("a saved block is JSON");
    block["extra"].clone()
}

/// A label saved under `"label"`, `"Untitled"` by default, holding `initial`.
fn label(parts: &mut Parts, initial: &str) -> Value<String> {
    let label = Value::new(initial.to_owned(), parts.set());
    parts
        .save("label", &label, "Untitled".to_owned())
        .expect("a free key");
    label
}

#[test]
fn a_saved_value_loads_into_a_second_instance() {
    let mut first = parts();
    let first_driver = Driver::of(&first);
    let default = View {
        zoom: 1.0,
        panel: "mixer".to_owned(),
    };
    let view = Value::new(default.clone(), first.set());
    first
        .save("view", &view, default.clone())
        .expect("a free key");
    let moved = View {
        zoom: 2.5,
        panel: "sends".to_owned(),
    };
    view.set(moved.clone());
    first_driver.tick();
    let block = first.saved().save();

    let mut second = parts();
    let second_driver = Driver::of(&second);
    let reopened = Value::new(default.clone(), second.set());
    second
        .save("view", &reopened, default.clone())
        .expect("a free key");
    second.saved().load(&block).expect("a saved block");
    assert_eq!(reopened.get(), default, "a load applies at the tick");
    second_driver.tick();
    assert_eq!(reopened.get(), moved);
    assert_eq!(second.saved().save(), block);
}

#[test]
fn a_block_saved_without_the_key_loads_the_default() {
    let older = parts().saved().save();

    let mut parts = parts();
    let driver = Driver::of(&parts);
    let label = label(&mut parts, "Drums");
    parts.saved().load(&older).expect("a saved block");
    driver.tick();
    assert_eq!(label.get(), "Untitled");
    assert_eq!(extra(&parts), json!({ "label": "Untitled" }));
}

#[test]
fn a_section_of_the_wrong_shape_fails_its_apply_and_stays_saved() {
    let mut parts = parts();
    let driver = Driver::of(&parts);
    let label = label(&mut parts, "Drums");
    let mut block: serde_json::Value =
        serde_json::from_slice(&parts.saved().save()).expect("a saved block is JSON");
    block["extra"]["label"] = json!(7);
    let block = serde_json::to_vec(&block).expect("JSON writes");

    parts.saved().load(&block).expect("a saved block");
    driver.tick();
    assert_eq!(label.get(), "Drums", "the value keeps what it held");
    assert_eq!(extra(&parts), json!({ "label": 7 }));
}

#[test]
fn a_save_holds_the_value_from_before_a_change_until_a_tick_or_a_sync() {
    let mut parts = parts();
    let driver = Driver::of(&parts);
    let label = label(&mut parts, "Untitled");

    label.set("Drums".to_owned());
    assert_eq!(extra(&parts), json!({ "label": "Untitled" }));
    driver.tick();
    assert_eq!(extra(&parts), json!({ "label": "Drums" }));

    label.set("Bass".to_owned());
    assert_eq!(extra(&parts), json!({ "label": "Drums" }));
    parts.sync();
    assert_eq!(extra(&parts), json!({ "label": "Bass" }));
}

#[test]
fn a_value_written_from_another_thread_is_captured_at_the_next_tick() {
    let mut parts = parts();
    let driver = Driver::of(&parts);
    let label = label(&mut parts, "Untitled");

    std::thread::scope(|scope| {
        scope.spawn(|| label.set("Drums".to_owned()));
    });
    assert_eq!(extra(&parts), json!({ "label": "Untitled" }));
    driver.tick();
    assert_eq!(extra(&parts), json!({ "label": "Drums" }));
}

thread_local! {
    /// How many times a [`Counted`] was written as JSON on this thread.
    static CAPTURES: Cell<u32> = const { Cell::new(0) };
}

/// A number that counts its captures.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(transparent)]
struct Counted(u32);

impl Serialize for Counted {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        CAPTURES.with(|captures| captures.set(captures.get() + 1));
        self.0.serialize(serializer)
    }
}

fn captures() -> u32 {
    CAPTURES.with(Cell::get)
}

#[test]
fn a_load_is_captured_once_by_its_apply_and_a_later_change_again() {
    let mut parts = parts();
    let driver = Driver::of(&parts);
    let count = Value::new(Counted(1), parts.set());
    parts.save("count", &count, Counted(0)).expect("a free key");
    let block = br#"{"seamStateVersion": 1, "parameters": {}, "extra": {"count": 2}}"#;

    let before = captures();
    parts.saved().load(block).expect("a saved block");
    driver.tick();
    assert_eq!(count.get(), Counted(2));
    assert_eq!(captures() - before, 1, "the apply's capture, and no other");
    driver.tick();
    assert_eq!(captures() - before, 1);

    count.set(Counted(3));
    driver.tick();
    assert_eq!(captures() - before, 2);
    assert_eq!(extra(&parts), json!({ "count": 3 }));
}

#[test]
fn a_second_save_under_one_key_is_refused() {
    let mut parts = parts();
    let driver = Driver::of(&parts);
    let label = label(&mut parts, "Drums");
    let other = Value::new("Bass".to_owned(), parts.set());

    let refused = parts.save("label", &other, "Untitled".to_owned());
    assert_eq!(refused, Err(SessionError::KeyTaken("label".to_owned())));
    other.set("Keys".to_owned());
    driver.tick();
    assert_eq!(extra(&parts), json!({ "label": "Drums" }));
    assert_eq!(label.get(), "Drums");
}

/// A participant the plugin registered by hand that reaches back into the
/// parts from its apply: it saves a value there, and syncs.
struct Reaching {
    parts: Rc<RefCell<Parts>>,
    label: Value<String>,
}

impl Participant for Reaching {
    fn capture(&self) -> Section {
        Section::Bool(true)
    }

    fn apply(&mut self, section: Option<&Section>) -> Result<(), ApplyError> {
        if section.is_some() {
            let mut parts = self.parts.borrow_mut();
            parts
                .save("label", &self.label, "Untitled".to_owned())
                .expect("a free key");
            parts.sync();
        }
        Ok(())
    }
}

#[test]
fn a_participant_may_save_and_sync_from_an_apply_the_tick_runs() {
    let parts = Rc::new(RefCell::new(parts()));
    let driver = Driver::of(&parts.borrow());
    let label = Value::new("Untitled".to_owned(), parts.borrow().set());
    let reaching = Rc::new(RefCell::new(Reaching {
        parts: Rc::clone(&parts),
        label: label.clone(),
    }));
    let _registration = parts
        .borrow()
        .session()
        .register("reaching", &reaching)
        .expect("a free key");

    let block = br#"{"seamStateVersion": 1, "parameters": {},
        "extra": {"reaching": true, "label": "Drums"}}"#;
    parts.borrow().saved().load(block).expect("a saved block");
    driver.tick();
    assert_eq!(label.get(), "Drums", "saved inside the apply, and loaded");
    assert_eq!(
        extra(&parts.borrow()),
        json!({ "reaching": true, "label": "Drums" })
    );

    label.set("Bass".to_owned());
    driver.tick();
    assert_eq!(extra(&parts.borrow())["label"], "Bass", "and kept saved");
}

#[cfg(debug_assertions)]
#[test]
fn a_value_serde_cannot_write_as_json_is_a_contract_panic() {
    use std::collections::BTreeMap;

    let mut parts = parts();
    // JSON's object keys are strings, and a pair is not one.
    let pairs = Value::new(BTreeMap::from([((1_u8, 2_u8), 3_u8)]), parts.set());
    seam_testkit::contract::expect_contract_panic("serde cannot write a saved value", || {
        let _ = parts.save("pairs", &pairs, BTreeMap::new());
    });
}
