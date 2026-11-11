//! The plugin trait: a plugin with main-thread state that is not `Send`.

use std::cell::Cell;
use std::rc::Rc;
use std::sync::Arc;

use seam_host::{
    AuCodes, Block, Category, Layout, Parts, Plugin, PluginInfo, ProcessSetup, Processor, Status,
};
use seam_params::{Bool, BoolDecl, Declarer, Kind, ParamDecl, ParamSet, Params};
use seam_session::Session;
use serde_json::json;

const PARAMS: &[ParamDecl] = &[ParamDecl::new("on", "On", Kind::Bool, 1.0)];

#[derive(Debug, Clone)]
struct OnParams {
    on: Bool,
}

impl Params for OnParams {
    fn declare(p: &mut Declarer) -> Self {
        OnParams {
            on: p.add(BoolDecl::new("on", "On", true)),
        }
    }
}

struct Silent;

impl Processor for Silent {
    fn process(&mut self, _: Block<'_, '_>) -> Status {
        Status::Sleep
    }
}

/// Owns main-thread state that is not `Send`: its parts and an `Rc`.
struct Minimal {
    parts: Parts,
    params: OnParams,
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
    type Params = OnParams;
    type Processor = Silent;

    fn new(params: OnParams, parts: Parts) -> Self {
        Minimal {
            parts,
            params,
            ticks: Rc::default(),
        }
    }

    fn activate(&mut self, _: &ProcessSetup) -> Silent {
        Silent
    }

    fn idle(&mut self) {
        self.ticks.set(self.ticks.get() + 1);
        self.parts.tick();
    }
}

#[test]
fn a_plugin_owning_main_thread_state_saves_and_loads_from_another_thread() {
    // The plugin stays on the main thread, and the host's state calls go
    // through the saved state its parts hold.
    let (params, parts) = Parts::for_plugin::<Minimal>().expect("valid declarations");
    let mut plugin = Minimal::new(params, parts);
    let saved = Arc::clone(plugin.parts.saved());
    assert!(
        Arc::ptr_eq(saved.set(), plugin.parts.set()),
        "one set: the host's, the saved block's and the plugin's"
    );
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
    assert!(
        !plugin.params.on.get(),
        "the load wrote the parameter the plugin holds"
    );
    plugin.idle();
    assert_eq!(plugin.ticks.get(), 1);
}

#[test]
fn a_mapping_learned_after_the_session_ticks_waits_a_tick_for_the_save() {
    // Why `Parts::tick` polls learn before it ticks the session.
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

/// A plugin that takes MIDI, for parts with learn on. Its parts are used
/// with no plugin built.
struct Listening;

impl Plugin for Listening {
    const INFO: PluginInfo = PluginInfo {
        name: "Listening",
        id: "dev.seam.listening",
        midi_input: true,
        ..Minimal::INFO
    };
    type Params = OnParams;
    type Processor = Silent;

    fn new(_: OnParams, _: Parts) -> Self {
        Listening
    }

    fn activate(&mut self, _: &ProcessSetup) -> Silent {
        Silent
    }
}

/// A section of the plugin's own: a zoom level the session restores.
struct Zoom(f64);

impl seam_session::Participant for Zoom {
    fn capture(&self) -> seam_session::Section {
        json!(self.0)
    }

    fn apply(
        &mut self,
        section: Option<&seam_session::Section>,
    ) -> Result<(), seam_session::ApplyError> {
        self.0 = section.and_then(serde_json::Value::as_f64).unwrap_or(1.0);
        Ok(())
    }
}

#[test]
fn one_tick_applies_a_restore_saves_a_learned_mapping_and_records_a_gesture() {
    use std::cell::RefCell;

    use seam_learn::{Config, SESSION_KEY, Takes};
    use seam_ump::{ChannelVoice, EventList, Voice};
    use seam_undo::Bound;

    let (params, mut parts) = Parts::for_plugin::<Listening>().expect("valid declarations");
    let history = parts.undo(Bound::default()).expect("no other history");
    parts
        .midi_learn(Config::default())
        .expect("learn's key is free");
    let zoom = Rc::new(RefCell::new(Zoom(1.0)));
    let _registration = parts.session().register("zoom", &zoom).expect("a free key");
    let saved = Arc::clone(parts.saved());

    // A host's load: the session applies at the next tick.
    let block = json!({
        "seamStateVersion": 1,
        "parameters": {},
        "extra": {"zoom": 2.0},
    })
    .to_string();
    saved.load(block.as_bytes()).expect("a valid block");
    assert_eq!(zoom.borrow().0, 1.0, "nothing applies before the tick");
    assert!(parts.tick().is_none());
    assert_eq!(zoom.borrow().0, 2.0, "the tick applied the restore");

    // A drag, and a controller learned in the same tick.
    let on = params.on.id();
    parts.params().write(on, 0.0);
    let mut audio = parts.learn_audio();
    parts.learn().arm(on, Takes::CONTROLLER).expect("a target");
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
    assert!(parts.tick().is_some(), "the tick committed the mapping");
    assert_eq!(history.len(), 1, "the drag is one undo step");
    let block: serde_json::Value = serde_json::from_slice(&saved.save()).expect("JSON");
    assert_eq!(
        block["extra"][SESSION_KEY]["mappings"][0]["target"], "on",
        "the same tick's save holds the mapping"
    );
}

#[test]
#[should_panic(expected = "MIDI learn needs MIDI input")]
fn learn_is_refused_to_a_plugin_that_takes_no_midi() {
    let (_, mut parts) = Parts::for_plugin::<Minimal>().expect("valid declarations");
    let _ = parts.midi_learn(seam_learn::Config::default());
}

#[test]
fn undo_is_refused_while_the_plugin_s_own_history_is_alive() {
    use seam_undo::{Bound, History, HistoryError};

    let (_, mut parts) = Parts::for_plugin::<Minimal>().expect("valid declarations");
    let own = History::new(parts.params(), Bound::default()).expect("the first history");
    assert_eq!(
        parts.undo(Bound::default()).err(),
        Some(HistoryError::HistoryExists)
    );
    drop(own);
    let _history = parts.undo(Bound::default()).expect("no other history");
    assert!(
        parts.undo(Bound::default()).is_ok(),
        "a second call hands back the same history"
    );
}
