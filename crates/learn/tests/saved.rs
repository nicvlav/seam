//! The saved mappings: the `midiLearn` section, restore, and session wiring.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use seam_learn::{
    Config, Control, Curve, Learn, LearnAudio, Mapping, NrpnReservation, SESSION_KEY, Source,
    Target, TargetId,
};
use seam_params::{Kind, ParamDecl, ParamId, ParamSet, ParamSetBuilder};
use seam_session::Session;
use serde_json::json;

fn set() -> Arc<ParamSet> {
    let mut builder = ParamSetBuilder::new();
    for key in ["cutoff", "gain", "pan"] {
        builder
            .add(ParamDecl::new(
                key,
                key,
                Kind::continuous(0.0, 1.0, 0.0, None),
                0.5,
            ))
            .expect("valid");
    }
    builder.build()
}

struct Quiet;

impl Target for Quiet {
    fn begin(&mut self) {}
    fn apply(&mut self, _: f64) {}
    fn end(&mut self) {}
}

fn learn_with(config: Config) -> (Arc<ParamSet>, Learn, LearnAudio) {
    let set = set();
    let (learn, audio) = Learn::new(&set, config);
    (set, learn, audio)
}

fn id(set: &ParamSet, key: &str) -> ParamId {
    set.id(key).expect("declared")
}

#[test]
fn the_section_is_schema_1_sorted_by_key_with_the_identity_curve_left_out() {
    let (set, mut learn, _audio) = learn_with(Config::default());
    learn
        .map(
            id(&set, "pan"),
            Mapping::new(Source {
                control: Control::Note(60),
                group: None,
                channel: Some(3),
            }),
        )
        .expect("legal");
    let mut nrpn = Mapping::new(Source {
        control: Control::Nrpn(300),
        group: Some(2),
        channel: None,
    });
    nrpn.coarse = true;
    nrpn.curve = Curve::new(2.0, 0.25, 0.75).expect("valid").inverted();
    learn.map(id(&set, "cutoff"), nrpn).expect("legal");
    learn
        .map(id(&set, "gain"), Mapping::new(Source::any(Control::Cc(7))))
        .expect("legal");
    // A custom target sorts by its key, not after the parameters.
    let custom = learn.register("amp.view", Box::new(Quiet)).expect("free");
    learn
        .map(custom, Mapping::new(Source::any(Control::Cc(8))))
        .expect("legal");
    assert_eq!(
        learn.save(),
        json!({"schema": 1, "mappings": [
            {"target": "amp.view", "kind": "cc", "group": -1, "channel": -1, "number": 8},
            {"target": "cutoff", "kind": "nrpn", "group": 2, "channel": -1, "number": 300,
             "coarse": true,
             "curve": {"invert": true, "skew": 2.0, "low": 0.25, "high": 0.75}},
            {"target": "gain", "kind": "cc", "group": -1, "channel": -1, "number": 7},
            {"target": "pan", "kind": "note", "group": -1, "channel": 3, "number": 60},
        ]})
    );
}

#[test]
fn save_restore_save_gives_the_same_section_and_routes_without_writing_values() {
    // A restore restores routing, never values.
    let (set, mut learn, _audio) = learn_with(Config::default());
    let mut mapping = Mapping::new(Source::any(Control::Nrpn(5)));
    mapping.curve = Curve::new(0.7, 0.1, 0.9).expect("valid");
    learn.map(id(&set, "gain"), mapping).expect("legal");
    let saved = learn.save();

    let (other, mut restored, mut audio) = learn_with(Config::default());
    restored.restore(Some(&saved)).expect("schema 1");
    assert_eq!(restored.save(), saved);
    assert_eq!(restored.mapping(id(&other, "gain")), Some(&mapping));
    assert_eq!(audio.refresh_slots(), 1, "published");
    assert_eq!(other.value(id(&other, "gain")), 0.5, "no value written");
}

#[test]
fn a_restore_replaces_every_mapping_and_drops_a_bad_entry_on_its_own() {
    // A restore is total and forgiving.
    let mut config = Config::default();
    config.reserved_nrpns = vec![NrpnReservation::new(None, None, 1000, 1000).expect("valid")];
    let (set, mut learn, mut audio) = learn_with(config);
    learn
        .map(id(&set, "pan"), Mapping::new(Source::any(Control::Cc(10))))
        .expect("legal");
    let section = json!({"schema": 1, "mappings": [
        {"target": "gain", "kind": "cc", "group": -1, "channel": -1, "number": 7},
        {"target": "cutoff", "kind": "pitch", "group": -1, "channel": -1, "number": 1},
        {"target": "cutoff", "kind": "cc", "group": 16, "channel": -1, "number": 1},
        {"target": "cutoff", "kind": "cc", "group": -1, "channel": -1, "number": 300},
        {"target": "cutoff", "kind": "cc", "group": -1, "channel": -1, "number": 6},
        {"target": "cutoff", "kind": "nrpn", "group": -1, "channel": -1, "number": 1000},
        {"target": "cutoff", "kind": "cc", "number": 2, "curve": {"skew": 0.0}},
        "not an entry",
    ]});
    learn.restore(Some(&section)).expect("schema 1");
    let mapped: Vec<TargetId> = learn.mappings().map(|(target, _)| target).collect();
    assert_eq!(
        mapped,
        [TargetId::Param(id(&set, "gain"))],
        "pan's mapping replaced"
    );
    assert_eq!(audio.refresh_slots(), 1);

    learn.restore(None).expect("defaults");
    assert!(learn.is_empty());
    assert_eq!(audio.refresh_slots(), 0);
}

#[test]
fn a_section_of_another_schema_is_refused_and_clears_the_routing() {
    // The previous project's mappings must not keep routing in a project
    // whose section this build cannot read.
    let (set, mut learn, mut audio) = learn_with(Config::default());
    learn
        .map(id(&set, "gain"), Mapping::new(Source::any(Control::Cc(7))))
        .expect("legal");
    assert_eq!(audio.refresh_slots(), 1);
    assert!(
        learn
            .restore(Some(&json!({"schema": 2, "mappings": []})))
            .is_err()
    );
    assert!(learn.is_empty());
    assert_eq!(audio.refresh_slots(), 0, "the audio side stopped routing");

    learn
        .map(id(&set, "gain"), Mapping::new(Source::any(Control::Cc(7))))
        .expect("legal");
    assert!(learn.restore(Some(&json!([1]))).is_err());
    assert!(learn.is_empty());
    assert_eq!(audio.refresh_slots(), 0);
}

#[test]
fn an_entry_for_a_target_not_registered_yet_is_held_saved_back_and_lands_at_registration() {
    // Saved back unchanged, unknown fields included, and a
    // held entry takes no ceiling slot.
    let mut config = Config::default();
    config.max_mappings = 1;
    let (_set, mut learn, _audio) = learn_with(config);
    let held = json!({"target": "view.zoom", "kind": "cc", "group": -1, "channel": 0,
                      "number": 20, "newer": [1, 2]});
    let section = json!({"schema": 1, "mappings": [
        {"target": "gain", "kind": "cc", "group": -1, "channel": -1, "number": 7},
        held,
    ]});
    let mut with_bad = section.clone();
    with_bad["mappings"]
        .as_array_mut()
        .expect("a list")
        .push(json!({"target": "zz.bad", "kind": "cc", "group": 16, "channel": 0, "number": 1}));
    learn.restore(Some(&with_bad)).expect("schema 1");
    assert_eq!(learn.len(), 1, "the held entry takes no slot");
    assert_eq!(
        learn.save(),
        section,
        "saved back unchanged; an entry out of range is not held"
    );

    // The ceiling is full, so the landing is refused and the entry drops.
    let zoom = learn.register("view.zoom", Box::new(Quiet)).expect("free");
    assert_eq!(learn.mapping(zoom), None);
    assert_eq!(learn.save()["mappings"].as_array().map(Vec::len), Some(1));

    // With room, it lands under the same checks.
    let (_set, mut learn, mut audio) = learn_with(Config::default());
    learn.restore(Some(&section)).expect("schema 1");
    let zoom = learn.register("view.zoom", Box::new(Quiet)).expect("free");
    assert_eq!(
        learn.mapping(zoom).map(|mapping| mapping.source),
        Some(Source {
            control: Control::Cc(20),
            group: None,
            channel: Some(0),
        })
    );
    assert_eq!(audio.refresh_slots(), 2);
    assert_eq!(learn.key(zoom), Some("view.zoom"));
}

#[test]
fn attached_to_a_session_learn_saves_each_change_and_restores_through_it() {
    // A mapping change made inside learn's own method reaches the
    // session at its next tick.
    let set = set();
    let session = Session::new(&set);
    let ui = session.take_ui().expect("the first UI handle");
    let (learn, mut audio) = Learn::new(&set, Config::default());
    let learn = Rc::new(RefCell::new(learn));
    Learn::attach(&learn, &ui).expect("free");
    assert_eq!(
        session.save(),
        Some(json!({SESSION_KEY: {"schema": 1, "mappings": []}}))
    );
    learn
        .borrow_mut()
        .map(id(&set, "gain"), Mapping::new(Source::any(Control::Cc(7))))
        .expect("legal");
    ui.tick();
    let saved = session.save().expect("a section");
    assert_eq!(saved[SESSION_KEY]["mappings"][0]["target"], "gain");

    session.restore(Some(&json!({SESSION_KEY: {"schema": 1, "mappings": [
        {"target": "pan", "kind": "cc", "group": -1, "channel": -1, "number": 9},
    ]}})));
    ui.tick();
    let mapped: Vec<TargetId> = learn
        .borrow()
        .mappings()
        .map(|(target, _)| target)
        .collect();
    assert_eq!(mapped, [TargetId::Param(id(&set, "pan"))]);
    assert_eq!(audio.refresh_slots(), 1);
    assert_eq!(
        session.save().expect("a section")[SESSION_KEY]["mappings"][0]["target"],
        "pan"
    );
}

#[test]
fn a_session_restore_of_another_schema_stops_the_old_routing_and_keeps_the_section() {
    let set = set();
    let session = Session::new(&set);
    let ui = session.take_ui().expect("the first UI handle");
    let (learn, mut audio) = Learn::new(&set, Config::default());
    let learn = Rc::new(RefCell::new(learn));
    Learn::attach(&learn, &ui).expect("free");
    learn
        .borrow_mut()
        .map(id(&set, "gain"), Mapping::new(Source::any(Control::Cc(7))))
        .expect("legal");
    ui.tick();
    assert_eq!(audio.refresh_slots(), 1);

    let newer = json!({SESSION_KEY: {"schema": 2, "mappings": []}});
    session.restore(Some(&newer));
    ui.tick();
    assert!(learn.borrow().is_empty());
    assert_eq!(audio.refresh_slots(), 0);
    assert_eq!(
        session.save(),
        Some(newer),
        "the newer build's section stays"
    );
}

#[test]
fn a_refused_section_reports_no_change_to_the_session() {
    let set = set();
    let session = Session::new(&set);
    let ui = session.take_ui().expect("the first UI handle");
    let (learn, _audio) = Learn::new(&set, Config::default());
    let learn = Rc::new(RefCell::new(learn));
    Learn::attach(&learn, &ui).expect("free");
    learn
        .borrow_mut()
        .map(id(&set, "gain"), Mapping::new(Source::any(Control::Cc(7))))
        .expect("legal");
    ui.tick();
    let saved = session.save();

    let newer = json!({"schema": 2, "mappings": []});
    assert!(learn.borrow_mut().restore(Some(&newer)).is_err());
    ui.tick();
    assert!(learn.borrow().is_empty());
    assert_eq!(session.save(), saved, "nothing new to save");
}

struct Logged {
    log: Rc<RefCell<Vec<&'static str>>>,
}

impl Target for Logged {
    fn begin(&mut self) {
        self.log.borrow_mut().push("begin");
    }
    fn apply(&mut self, _: f64) {
        self.log.borrow_mut().push("apply");
    }
    fn end(&mut self) {
        self.log.borrow_mut().push("end");
    }
}

#[test]
fn a_restore_ends_the_arm_drops_old_held_entries_and_closes_bursts() {
    // A restore is a complete assignment of the mappings.
    let (set, mut learn, mut audio) = learn_with(Config::default());
    let log = Rc::new(RefCell::new(Vec::new()));
    let zoom = learn
        .register(
            "view.zoom",
            Box::new(Logged {
                log: Rc::clone(&log),
            }),
        )
        .expect("free");
    learn
        .map(zoom, Mapping::new(Source::any(Control::Cc(7))))
        .expect("legal");
    let mut events = seam_ump::EventList::with_capacity(1);
    let cc = |value| seam_ump::ChannelVoice {
        group: 0,
        channel: 0,
        voice: seam_ump::Voice::Midi1ControlChange {
            controller: 7,
            value,
        },
    };
    events.push(cc(127).event(0));
    audio.process(&events);
    learn.poll();
    assert_eq!(*log.borrow(), ["begin", "apply"]);
    // A value staged before the restore, which no poll after it applies.
    audio.process(&events);
    learn
        .arm(id(&set, "gain"), seam_learn::Takes::CONTROLLER)
        .expect("known");

    let held = json!({"schema": 1, "mappings": [
        {"target": "later", "kind": "cc", "group": -1, "channel": -1, "number": 9},
    ]});
    learn.restore(Some(&held)).expect("schema 1");
    assert_eq!(
        *log.borrow(),
        ["begin", "apply", "end"],
        "the burst closed once"
    );
    assert_eq!(learn.armed(), None, "the arm ended");
    audio.process(&events);
    learn.poll();
    assert_eq!(log.borrow().len(), 3, "the unmapped target is not driven");

    learn
        .restore(Some(&json!({"schema": 1, "mappings": []})))
        .expect("schema 1");
    assert_eq!(
        learn.save(),
        json!({"schema": 1, "mappings": []}),
        "the held entry went"
    );
}

#[test]
fn a_held_entry_landing_or_dropping_at_registration_reaches_the_session() {
    let set = set();
    let session = Session::new(&set);
    let ui = session.take_ui().expect("the first UI handle");
    let (learn, _audio) = Learn::new(&set, Config::default());
    let learn = Rc::new(RefCell::new(learn));
    Learn::attach(&learn, &ui).expect("free");
    // Held as saved, unknown field included; once live, written from the
    // mapping, without it.
    let held = json!({"target": "view.zoom", "kind": "cc", "group": -1, "channel": -1,
                      "number": 20, "newer": 1});
    learn
        .borrow_mut()
        .restore(Some(&json!({"schema": 1, "mappings": [held]})))
        .expect("schema 1");
    ui.tick();
    learn
        .borrow_mut()
        .register("view.zoom", Box::new(Quiet))
        .expect("free");
    ui.tick();
    let saved = session.save().expect("a section");
    assert_eq!(saved[SESSION_KEY]["mappings"][0]["target"], "view.zoom");
    assert_eq!(
        saved[SESSION_KEY]["mappings"][0].get("newer"),
        None,
        "the session heard the entry land"
    );
    assert_eq!(learn.borrow().len(), 1, "it landed");
}

#[test]
fn a_fresh_audio_side_reads_the_current_mappings_and_the_old_one_stops() {
    // Each activation's processor gets its own audio side.
    let (set, mut learn, mut old) = learn_with(Config::default());
    let gain = id(&set, "gain");
    learn
        .map(gain, Mapping::new(Source::any(Control::Cc(7))))
        .expect("legal");
    let mut fresh = learn.replace_audio();
    assert_eq!(fresh.refresh_slots(), 1, "the current mappings");
    learn
        .map(id(&set, "pan"), Mapping::new(Source::any(Control::Cc(8))))
        .expect("legal");
    assert_eq!(fresh.refresh_slots(), 2);
    assert_eq!(
        old.refresh_slots(),
        1,
        "the old side sees no later mapping change"
    );
}

#[test]
fn attaching_to_a_session_made_from_another_set_is_refused() {
    // The plugin's saved block would never hold the mappings.
    let other = ParamSetBuilder::new().build();
    let session = Session::new(&other);
    let ui = session.take_ui().expect("the first UI handle");
    let (learn, _audio) = Learn::new(&set(), Config::default());
    let learn = Rc::new(RefCell::new(learn));
    seam_testkit::contract::expect_contract_panic("made from another ParamSet", || {
        let _ = Learn::attach(&learn, &ui);
    });
    assert!(session.save().is_none(), "nothing registered");
}
