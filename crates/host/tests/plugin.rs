//! The plugin trait: a plugin with main-thread state that is not `Send`.

use std::cell::Cell;
use std::rc::Rc;
use std::sync::Arc;

use seam_host::{
    AuCodes, Block, Category, Layout, Plugin, PluginInfo, ProcessSetup, Processor, SavedState,
    Status,
};
use seam_params::{Kind, ParamDecl, ParamSet};
use seam_session::{Session, SessionUi};
use serde_json::json;

const PARAMS: &[ParamDecl] = &[ParamDecl::new("on", "On", Kind::Bool, 1.0)];

struct Silent;

impl Processor for Silent {
    fn process(&mut self, _: Block<'_, '_>) -> Status {
        Status::Sleep
    }
}

/// Owns main-thread state that is not `Send`: the session's UI handle and
/// an `Rc`.
struct Minimal {
    saved: Arc<SavedState>,
    _session: SessionUi,
    ticks: Rc<Cell<u32>>,
}

impl Plugin for Minimal {
    const INFO: PluginInfo = PluginInfo {
        name: "Minimal",
        vendor: "Seam",
        url: "",
        email: "",
        description: "",
        version: "0.1.0",
        id: "dev.seam.minimal",
        vst3_class_id: [0; 16],
        au_codes: AuCodes {
            subtype: *b"mini",
            manufacturer: *b"Seam",
        },
        category: Category::Effect,
        features: &[],
        layouts: &[Layout::STEREO],
        midi_input: false,
        midi_output: false,
    };
    const PARAMS: &'static [ParamDecl] = PARAMS;
    type Processor = Silent;

    fn new() -> Self {
        let set = ParamSet::from_decls(PARAMS.iter().copied()).expect("valid");
        let session = Session::new(&set);
        Minimal {
            saved: Arc::new(SavedState::new(&set, &session)),
            _session: session.take_ui().expect("the first UI handle"),
            ticks: Rc::default(),
        }
    }

    fn saved(&self) -> Arc<SavedState> {
        Arc::clone(&self.saved)
    }

    fn activate(&mut self, _: &ProcessSetup) -> Silent {
        Silent
    }

    fn idle(&mut self) {
        self.ticks.set(self.ticks.get() + 1);
    }
}

#[test]
fn a_plugin_owning_main_thread_state_saves_and_loads_from_another_thread() {
    // The plugin stays on the main thread, and the host's state calls go
    // through the handle it hands the exporter.
    let mut plugin = Minimal::new();
    let saved = plugin.saved();
    assert!(Arc::ptr_eq(&saved, &plugin.saved()));
    // The exporter's way to the plugin's one set.
    let on = saved.set().id("on").expect("declared");
    let block = json!({"seamStateVersion": 1, "parameters": {"on": false}}).to_string();
    let bytes = std::thread::scope(|scope| {
        scope
            .spawn(|| {
                assert!(saved.load(block.as_bytes()).is_ok());
                saved.save()
            })
            .join()
            .expect("the host's state thread")
    });
    let read: serde_json::Value = serde_json::from_slice(&bytes).expect("JSON");
    assert_eq!(read["parameters"]["on"], false);
    assert_eq!(
        saved.set().value(on),
        0.0,
        "the load wrote the set it hands out"
    );
    plugin.idle();
    assert_eq!(plugin.ticks.get(), 1);
}

#[test]
fn a_mapping_learned_after_the_session_ticks_waits_a_tick_for_the_save() {
    // Why `Plugin::idle` polls learn before it ticks the session.
    use std::cell::RefCell;

    use seam_learn::{Config, Learn, SESSION_KEY, Takes};
    use seam_ump::{ChannelVoice, EventList, Voice};

    let set = ParamSet::from_decls(PARAMS.iter().copied()).expect("valid");
    let session = Session::new(&set);
    let ui = session.take_ui().expect("the first UI handle");
    let (learn, mut audio) = Learn::new(&set, Config::default());
    let learn = Rc::new(RefCell::new(learn));
    Learn::attach(&learn, &ui).expect("learn's key is free");
    let on = set.id("on").expect("declared");
    learn
        .borrow_mut()
        .arm(on, Takes::CONTROLLER)
        .expect("a target");
    let voice = Voice::Midi1ControlChange {
        controller: 74,
        value: 64,
    };
    let mut events = EventList::with_capacity(1);
    events.push(
        ChannelVoice {
            group: 0,
            channel: 0,
            voice,
        }
        .event(0),
    );
    audio.process(&events);
    // The first mapping's target, as the next save would write it.
    let saved = || {
        session.save().and_then(|saved| {
            saved[SESSION_KEY]["mappings"][0]["target"]
                .as_str()
                .map(str::to_owned)
        })
    };

    ui.tick();
    let _learned = learn.borrow_mut().poll();
    assert_eq!(saved(), None, "the tick came first");
    ui.tick();
    assert_eq!(saved().as_deref(), Some("on"), "the next one captures it");
}
