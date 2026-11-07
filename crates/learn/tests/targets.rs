//! Targets learn writes on the UI thread: bursts, detaching and holding.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::Arc;

use seam_learn::{
    Config, Control, CustomId, Learn, LearnAudio, LearnError, Learned, Mapping, Source, Takes,
    Target, TargetId,
};
use seam_params::{Kind, ParamDecl, ParamSet, ParamSetBuilder};
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
