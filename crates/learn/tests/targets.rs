//! Targets learn writes on the UI thread: bursts, detaching and holding.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::Arc;

use seam_learn::{
    Config, Control, CustomId, Learn, LearnAudio, LearnError, Learned, Mapping, SESSION_KEY,
    Source, Takes, Target, TargetId,
};
use seam_params::{Declarer, Float, FloatDecl, Kind, ParamDecl, ParamSet, ParamSetBuilder};
use seam_session::Session;
use seam_ump::{ChannelVoice, EventList, Voice};

fn set() -> Arc<ParamSet> {
    let mut builder = ParamSetBuilder::new();
    builder
        .add(ParamDecl::new(
            "gain",
            "Gain",
            Kind::continuous(0.0, 1.0, 0.0, None),
            0.5,
        ))
        .expect("valid");
    builder.build()
}

/// A target that logs what learn does to it.
struct Logged {
    log: Rc<RefCell<Vec<String>>>,
    held: Rc<Cell<bool>>,
}

impl Target for Logged {
    fn begin(&mut self) {
        self.log.borrow_mut().push("begin".into());
    }
    fn apply(&mut self, normalized: f64) {
        self.log.borrow_mut().push(format!("apply {normalized:.3}"));
    }
    fn end(&mut self) {
        self.log.borrow_mut().push("end".into());
    }
    fn held(&self) -> bool {
        self.held.get()
    }
}

impl Drop for Logged {
    fn drop(&mut self) {
        self.log.borrow_mut().push("dropped".into());
    }
}

struct Rig {
    set: Arc<ParamSet>,
    learn: Learn,
    audio: LearnAudio,
    log: Rc<RefCell<Vec<String>>>,
    held: Rc<Cell<bool>>,
    target: CustomId,
}

fn rig() -> Rig {
    let set = set();
    let (mut learn, audio) = Learn::new(&set, Config::default());
    let (log, held) = (Rc::default(), Rc::new(Cell::new(false)));
    let target = learn
        .register(
            "view.zoom",
            Box::new(Logged {
                log: Rc::clone(&log),
                held: Rc::clone(&held),
            }),
        )
        .expect("free");
    Rig {
        set,
        learn,
        audio,
        log,
        held,
        target,
    }
}

impl Rig {
    fn block(&mut self, values: &[u8]) {
        let mut events = EventList::with_capacity(values.len());
        for (offset, &value) in values.iter().enumerate() {
            let voice = ChannelVoice {
                group: 0,
                channel: 0,
                voice: Voice::Midi1ControlChange {
                    controller: 7,
                    value,
                },
            };
            events.push(voice.event(offset as u32));
        }
        self.audio.process(&events);
    }

    fn map(&mut self) {
        self.learn
            .map(self.target, Mapping::new(Source::any(Control::Cc(7))))
            .expect("legal");
    }

    fn taken(&self) -> Vec<String> {
        std::mem::take(&mut *self.log.borrow_mut())
    }
}

#[test]
fn a_registered_target_starts_unmapped_and_its_key_is_its_own() {
    let mut rig = rig();
    assert_eq!(rig.audio.refresh_slots(), 0);
    let other = || {
        Box::new(Logged {
            log: Rc::default(),
            held: Rc::default(),
        })
    };
    assert_eq!(rig.learn.register("", other()), Err(LearnError::EmptyKey));
    assert_eq!(
        rig.learn.register("gain", other()),
        Err(LearnError::KeyTaken)
    );
    assert_eq!(
        rig.learn.register("view.zoom", other()),
        Err(LearnError::KeyTaken)
    );
    rig.map();
    assert_eq!(rig.audio.refresh_slots(), 1);
}

#[test]
fn a_burst_is_one_gesture_of_the_newest_values_closed_after_15_idle_ticks() {
    // A sweep is one undo step.
    let mut rig = rig();
    rig.map();
    rig.block(&[0, 64, 127]);
    assert_eq!(rig.learn.poll(), None);
    assert_eq!(
        rig.taken(),
        ["begin", "apply 1.000"],
        "the newest value only"
    );
    rig.block(&[64]);
    rig.learn.poll();
    assert_eq!(rig.taken(), ["apply 0.500"], "the same gesture");
    for _ in 0..14 {
        rig.learn.poll();
    }
    assert!(rig.taken().is_empty(), "14 idle ticks keep it open");
    rig.learn.poll();
    assert_eq!(rig.taken(), ["end"]);
    rig.learn.poll();
    rig.block(&[0]);
    rig.learn.poll();
    assert_eq!(rig.taken(), ["begin", "apply 0.000"], "a new burst");
}

#[test]
fn while_the_user_holds_the_target_the_burst_closes_and_the_value_waits() {
    // A learned value must not write through the user's drag.
    let mut rig = rig();
    rig.map();
    rig.block(&[127]);
    rig.learn.poll();
    rig.held.set(true);
    rig.block(&[0]);
    rig.learn.poll();
    rig.learn.poll();
    assert_eq!(rig.taken(), ["begin", "apply 1.000", "end"]);
    rig.block(&[64]);
    rig.held.set(false);
    rig.learn.poll();
    assert_eq!(
        rig.taken(),
        ["begin", "apply 0.500"],
        "the newest, once let go"
    );
}

#[test]
fn an_unmapped_target_closes_its_burst_once_and_a_new_mapping_starts_empty() {
    let mut rig = rig();
    rig.map();
    rig.block(&[127]);
    rig.learn.poll();
    rig.block(&[0]);
    rig.learn.unmap(rig.target);
    rig.learn.poll();
    assert_eq!(rig.taken(), ["begin", "apply 1.000", "end"]);
    assert_eq!(rig.audio.refresh_slots(), 0);
    rig.map();
    rig.learn.poll();
    assert!(
        rig.taken().is_empty(),
        "the value staged before the unmap never lands"
    );
    rig.block(&[64]);
    rig.learn.poll();
    assert_eq!(rig.taken(), ["begin", "apply 0.500"]);
}

#[test]
fn a_detached_target_closes_its_burst_drops_and_is_unknown_after() {
    // A block in flight writes only the staging slot the old table
    // owns, which nothing reads.
    let mut rig = rig();
    rig.map();
    rig.block(&[127]);
    rig.learn.poll();
    assert_eq!(rig.taken(), ["begin", "apply 1.000"]);
    rig.learn.arm(rig.target, Takes::CONTROLLER).expect("known");
    assert!(rig.learn.detach(rig.target));
    assert_eq!(rig.taken(), ["end", "dropped"]);
    assert_eq!(rig.learn.armed(), None, "the arm on it ended");
    assert!(!rig.learn.detach(rig.target));
    rig.block(&[0]);
    rig.learn.poll();
    assert!(rig.taken().is_empty());
    assert_eq!(
        rig.learn.arm(rig.target, Takes::CONTROLLER),
        Err(LearnError::UnknownTarget(TargetId::Custom(rig.target)))
    );
}

#[test]
fn a_detached_target_keeps_its_mapping_until_its_key_registers_again() {
    // An editor that registers its targets when it opens and
    // detaches them when it closes keeps their mappings.
    let mut rig = rig();
    rig.map();
    let saved = rig.learn.save();
    assert!(rig.learn.detach(rig.target));
    assert_eq!(rig.learn.len(), 0, "no mapping drives a dropped target");
    assert_eq!(rig.audio.refresh_slots(), 0, "nor does the audio side");
    assert_eq!(rig.learn.save(), saved, "the entry is held, unchanged");
    rig.taken();

    let log = Rc::clone(&rig.log);
    let held = Rc::clone(&rig.held);
    let again = rig
        .learn
        .register("view.zoom", Box::new(Logged { log, held }))
        .expect("free");
    assert_eq!(
        rig.learn.mapping(again).map(|mapping| mapping.source),
        Some(Source::any(Control::Cc(7))),
        "the mapping landed again"
    );
    assert_eq!(rig.learn.save(), saved);
    rig.block(&[127]);
    rig.learn.poll();
    assert_eq!(rig.taken(), ["begin", "apply 1.000"]);

    assert!(rig.learn.unmap(again).is_some());
    assert!(rig.learn.detach(again));
    let gone = rig.learn.save();
    assert_eq!(gone["mappings"], serde_json::json!([]), "unmap removes it");
}

/// A target whose code panics when its burst closes.
struct PanicsAtEnd;

impl Target for PanicsAtEnd {
    fn begin(&mut self) {}
    fn apply(&mut self, _: f64) {}
    fn end(&mut self) {
        panic!("the plugin's end");
    }
}

#[test]
fn a_target_panicking_as_it_detaches_leaves_its_mapping_held_and_learn_usable() {
    // The table is settled before the target's own code runs.
    let mut rig = rig();
    let id = rig
        .learn
        .register("view.pan", Box::new(PanicsAtEnd))
        .expect("free");
    rig.learn
        .map(id, Mapping::new(Source::any(Control::Cc(7))))
        .expect("legal");
    rig.block(&[127]);
    rig.learn.poll();
    let detached = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| rig.learn.detach(id)));
    assert!(detached.is_err(), "the target's end panicked");
    let saved = rig.learn.save();
    assert_eq!(saved["mappings"][0]["target"], "view.pan", "held");
    assert_eq!(rig.learn.len(), 0);
    let gain = rig.set.id("gain").expect("declared");
    rig.learn
        .map(gain, Mapping::new(Source::any(Control::Cc(9))))
        .expect("learn still works");
}

#[test]
fn a_target_panicking_as_a_restore_closes_its_burst_leaves_learn_usable() {
    // The mappings clear before the target's own code runs, so no mapping is
    // left without the slot the next publish hands the audio thread.
    let mut rig = rig();
    let id = rig
        .learn
        .register("view.pan", Box::new(PanicsAtEnd))
        .expect("free");
    rig.learn
        .map(id, Mapping::new(Source::any(Control::Cc(7))))
        .expect("legal");
    rig.block(&[127]);
    rig.learn.poll();
    let restored =
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| rig.learn.restore(None)));
    assert!(restored.is_err(), "the target's end panicked");
    let gain = rig.set.id("gain").expect("declared");
    rig.learn
        .map(gain, Mapping::new(Source::any(Control::Cc(9))))
        .expect("learn still works");
    assert_eq!(
        rig.learn.mapping(id),
        None,
        "the old project's mapping went"
    );
    assert_eq!(rig.audio.refresh_slots(), 1);
}

/// One block of MIDI 1.0 control changes, as (controller, value) pairs.
fn controllers(changes: &[(u8, u8)]) -> EventList {
    let mut events = EventList::with_capacity(changes.len());
    for &(controller, value) in changes {
        let voice = Voice::Midi1ControlChange { controller, value };
        let voice = ChannelVoice {
            group: 0,
            channel: 0,
            voice,
        };
        events.push(voice.event(0));
    }
    events
}

#[test]
fn a_target_panicking_as_a_restore_leaves_the_audio_side_on_the_new_table() {
    // Target code runs after the publish and after the session hears of
    // the change, so the old project's mappings stop on the audio thread
    // and in the next save too, and a burst the panic left open closes at
    // the next poll.
    let set = set();
    let gain = set.id("gain").expect("declared");
    let session = Session::new(&set);
    let ui = session.take_ui().expect("the first UI handle");
    let (learn, mut audio) = Learn::new(&set, Config::default());
    let learn = Rc::new(RefCell::new(learn));
    Learn::attach(&learn, &ui).expect("free");
    let mut learning = learn.borrow_mut();
    let pan = learning
        .register("view.pan", Box::new(PanicsAtEnd))
        .expect("free");
    let log = Rc::new(RefCell::new(Vec::new()));
    let zoom = Logged {
        log: Rc::clone(&log),
        held: Rc::new(Cell::new(false)),
    };
    let zoom = learning
        .register("view.zoom", Box::new(zoom))
        .expect("free");
    for (target, controller) in [(TargetId::from(gain), 7), (pan.into(), 8), (zoom.into(), 9)] {
        let mapping = Mapping::new(Source::any(Control::Cc(controller)));
        learning.map(target, mapping).expect("legal");
    }
    audio.process(&controllers(&[(8, 127), (9, 127)]));
    learning.poll();
    assert_eq!(*log.borrow(), ["begin", "apply 1.000"], "both bursts open");
    log.borrow_mut().clear();
    drop(learning);
    ui.tick();
    let saved = session.save().expect("a section");
    assert_eq!(
        saved[SESSION_KEY]["mappings"].as_array().map(Vec::len),
        Some(3)
    );

    let mut learning = learn.borrow_mut();
    let restored =
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| learning.restore(None)));
    assert!(restored.is_err(), "the pan target's end panicked");
    assert!(learning.is_empty());
    audio.process(&controllers(&[(7, 127)]));
    assert_eq!(set.value(gain), 0.5, "controller 7 no longer routes");
    drop(learning);
    ui.tick();
    let saved = session.save().expect("a section");
    assert_eq!(saved[SESSION_KEY]["mappings"], serde_json::json!([]));
    assert!(log.borrow().is_empty(), "the panic came before zoom's end");
    learn.borrow_mut().poll();
    assert_eq!(*log.borrow(), ["end"], "the next poll closes it");
}

#[test]
fn a_target_panicking_as_it_unmaps_leaves_the_table_published_and_saved() {
    // Target code runs after the publish and after the session hears of
    // the change, so the audio side stops routing it and the next save
    // drops it.
    let set = set();
    let session = Session::new(&set);
    let ui = session.take_ui().expect("the first UI handle");
    let (learn, mut audio) = Learn::new(&set, Config::default());
    let learn = Rc::new(RefCell::new(learn));
    Learn::attach(&learn, &ui).expect("free");
    let pan = learn
        .borrow_mut()
        .register("view.pan", Box::new(PanicsAtEnd))
        .expect("free");
    let mapping = Mapping::new(Source::any(Control::Cc(8)));
    learn.borrow_mut().map(pan, mapping).expect("legal");
    audio.process(&controllers(&[(8, 127)]));
    learn.borrow_mut().poll();
    ui.tick();
    let saved = |session: &Session| session.save().expect("a section")[SESSION_KEY].clone();
    assert_eq!(saved(&session)["mappings"][0]["target"], "view.pan");

    let unmapped = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        learn.borrow_mut().unmap(pan)
    }));
    assert!(unmapped.is_err(), "the target's end panicked");
    assert_eq!(
        audio.refresh_slots(),
        0,
        "the audio side stopped routing it"
    );
    ui.tick();
    assert_eq!(saved(&session)["mappings"], serde_json::json!([]));
}

#[test]
fn a_detached_unmapped_target_leaves_nothing_held() {
    // Only a mapping is held.
    let mut rig = rig();
    let before = rig.learn.save();
    assert!(rig.learn.detach(rig.target));
    assert_eq!(rig.learn.save(), before);
}

#[test]
fn dropping_learn_closes_every_open_burst() {
    // Learn's end closes a burst.
    let mut rig = rig();
    rig.map();
    rig.block(&[127]);
    rig.learn.poll();
    let log = Rc::clone(&rig.log);
    log.borrow_mut().clear();
    drop(rig);
    assert_eq!(*log.borrow(), ["end", "dropped"]);
}

#[test]
fn a_custom_target_learns_like_a_parameter_and_each_keeps_its_tier() {
    // One source drives the parameter on the audio thread and the
    // custom target on the UI poll.
    let mut rig = rig();
    rig.learn.arm(rig.target, Takes::CONTROLLER).expect("known");
    rig.block(&[10]);
    let learned = rig.learn.poll();
    assert!(
        matches!(learned, Some(Learned::Mapped { target: TargetId::Custom(id), .. }) if id == rig.target)
    );
    let gain = rig.set.id("gain").expect("declared");
    rig.learn
        .map(gain, Mapping::new(Source::any(Control::Cc(7))))
        .expect("legal");
    rig.taken();
    rig.block(&[127]);
    assert_eq!(rig.set.value(gain), 1.0, "written in the block");
    assert!(
        rig.taken().is_empty(),
        "the custom target waits for the poll"
    );
    rig.learn.poll();
    assert_eq!(rig.taken(), ["begin", "apply 1.000"]);
}

#[test]
fn a_value_staged_while_the_user_holds_applies_once_they_let_go() {
    // Learn checks the hold before it takes the value, so a poll while
    // held loses nothing.
    let mut rig = rig();
    rig.map();
    rig.held.set(true);
    rig.block(&[64]);
    rig.learn.poll();
    rig.learn.poll();
    assert!(rig.taken().is_empty());
    rig.held.set(false);
    rig.learn.poll();
    assert_eq!(rig.taken(), ["begin", "apply 0.500"]);
}

#[test]
fn each_new_value_restarts_the_idle_count() {
    // A sweep with gaps shorter than 15 ticks is one burst.
    let mut rig = rig();
    rig.map();
    for value in [0, 64, 127] {
        rig.block(&[value]);
        for _ in 0..10 {
            rig.learn.poll();
        }
    }
    assert_eq!(
        rig.taken(),
        ["begin", "apply 0.000", "apply 0.500", "apply 1.000"]
    );
    // Nine idle ticks since the last value; six more make fifteen.
    for _ in 0..5 {
        rig.learn.poll();
    }
    assert!(rig.taken().is_empty());
    rig.learn.poll();
    assert_eq!(rig.taken(), ["end"]);
}

#[test]
fn a_re_pointed_target_never_applies_a_value_its_old_source_staged() {
    // Every mapping starts with nothing staged.
    let mut rig = rig();
    rig.map();
    rig.block(&[127]);
    rig.learn
        .map(rig.target, Mapping::new(Source::any(Control::Cc(8))))
        .expect("legal");
    rig.learn.poll();
    assert!(rig.taken().is_empty());
}

/// A parameter `set`'s instance makes at run time, as a plugin's parts do.
fn made(set: &ParamSet, key: &'static str) -> Float {
    let decl = FloatDecl::new(key, "Volume", -60.0..=12.0, 0.0);
    Declarer::run_time_one(set, Box::new(|_| None), decl).0
}

#[test]
fn typed_parameters_listed_and_made_at_run_time_are_targets() {
    let set = set();
    let gain: Float = set.bind("gain").expect("declared");
    let volume = made(&set, "volume");
    let (mut learn, _audio) = Learn::new(&set, Config::default());
    let mapping = Mapping::new(Source::any(Control::Cc(7)));

    // A listed typed parameter is the target its id names.
    learn.arm(&gain, Takes::CONTROLLER).expect("known");
    assert_eq!(learn.armed(), Some((gain.id().into(), Takes::CONTROLLER)));
    learn.map(&gain, mapping).expect("legal");
    assert_eq!(learn.mapping(gain.id()), Some(&mapping));
    assert_eq!(learn.key(&gain), Some("gain"));

    // One made at run time is a target of its own kind, known while it is
    // armed or mapped.
    assert_eq!(learn.key(&volume), None);
    learn.arm(&volume, Takes::CONTROLLER).expect("known");
    assert!(matches!(learn.armed(), Some((TargetId::RunTime(_), _))));
    assert_eq!(learn.key(&volume), Some("volume"));
    learn.map(&volume, mapping).expect("legal");
    assert_eq!(learn.armed(), None, "mapping the armed target ends the arm");
    assert_eq!(learn.mapping(&volume), Some(&mapping));
    let targets: Vec<_> = learn.mappings().map(|(target, _)| target).collect();
    assert!(matches!(
        targets[..],
        [TargetId::Param(_), TargetId::RunTime(_)]
    ));
    assert_eq!(learn.unmap(&volume), Some(mapping));
    assert_eq!(learn.mapping(&volume), None);
}

#[test]
fn a_run_time_mapping_is_left_out_of_learns_section_and_cleared_by_its_restore() {
    let set = set();
    let volume = made(&set, "volume");
    let (mut learn, _audio) = Learn::new(&set, Config::default());
    let mapping = Mapping::new(Source::any(Control::Cc(7)));
    learn
        .map(set.id("gain").expect("declared"), mapping)
        .expect("legal");
    learn.map(&volume, mapping).expect("legal");

    let saved = learn.save();
    let targets: Vec<_> = saved["mappings"]
        .as_array()
        .expect("an array")
        .iter()
        .map(|entry| entry["target"].clone())
        .collect();
    assert_eq!(targets, ["gain"], "its key alone names no one parameter");

    // Whatever saved it maps it again: a restore clears it with the rest.
    learn.restore(Some(&saved)).expect("readable");
    assert_eq!(learn.mapping(&volume), None, "cleared");
    assert_eq!(learn.len(), 1, "the listed mapping restored");
    learn.restore(None).expect("readable");
    assert!(learn.is_empty());
}

#[test]
fn another_instances_run_time_parameter_is_an_unknown_target() {
    let ours = set();
    let theirs = set();
    let volume = made(&theirs, "volume");
    let (mut learn, _audio) = Learn::new(&ours, Config::default());
    let mapping = Mapping::new(Source::any(Control::Cc(7)));

    assert!(matches!(
        learn.arm(&volume, Takes::CONTROLLER),
        Err(LearnError::UnknownTarget(TargetId::RunTime(_)))
    ));
    assert!(matches!(
        learn.map(&volume, mapping),
        Err(LearnError::UnknownTarget(_))
    ));
    assert_eq!(learn.mapping(&volume), None);
    assert_eq!(learn.key(&volume), None);
    assert_eq!(learn.unmap(&volume), None);
    assert!(learn.is_empty());
}

#[test]
fn learn_lets_a_parameter_go_at_the_poll_after_the_plugin_does() {
    let set = set();
    let (mut learn, mut audio) = Learn::new(&set, Config::default());
    let volume = made(&set, "volume");
    let armed = made(&set, "armed");
    learn
        .map(&volume, Mapping::new(Source::any(Control::Cc(7))))
        .expect("legal");
    learn.arm(&armed, Takes::CONTROLLER).expect("known");
    let copy = volume.clone();

    drop((volume, armed));
    assert!(learn.poll().is_none());
    assert_eq!(learn.len(), 1, "a clone still holds it");
    assert_eq!(learn.armed(), None, "the armed one let go");

    audio.process(&controllers(&[(7, 127)]));
    assert_eq!(copy.get(), 12.0, "a learned write reaches its store");
    drop(copy);
    assert_eq!(learn.len(), 1, "until the next poll");
    assert!(learn.poll().is_none());
    assert!(learn.is_empty());
}

#[test]
fn a_capture_for_a_parameter_the_plugin_let_go_of_maps_nothing() {
    let set = set();
    let (mut learn, mut audio) = Learn::new(&set, Config::default());
    let volume = made(&set, "volume");
    learn.arm(&volume, Takes::CONTROLLER).expect("known");
    drop(volume);

    audio.process(&controllers(&[(7, 127)]));
    assert!(learn.poll().is_none(), "no mapping to report");
    assert_eq!(learn.armed(), None);
    assert!(learn.is_empty());
}
