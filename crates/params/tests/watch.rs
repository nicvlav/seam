//! What surfaces show: what a watch reports during gestures, under racing
//! writers and across dropped watches.

use std::sync::Arc;

use proptest::prelude::*;
use proptest::test_runner::FileFailurePersistence;
use seam_params::{
    Edit, Gesture, Kind, ParamDecl, ParamId, ParamSet, ParamSetBuilder, Range, Reading, UiParams,
    Watch,
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

/// A host's 0..1 for a real value on the 0..10 range.
fn host(real: u8) -> f64 {
    f64::from(real) / 10.0
}

#[test]
fn a_watch_seeds_from_the_store_and_reports_the_latest_change_once() {
    let set = set_of(&["p"]);
    let ui = ui_of(&set);
    let audio = set.audio();
    let p = set.id("p").expect("declared");

    audio.host_write(p, host(3));
    let mut watch = ui.watch(p);
    assert_eq!(watch.id(), p);
    assert_eq!(watch.value(), 3.0, "the seed is the store");
    assert_eq!(watch.poll(), None, "the seed's write is already seen");

    audio.host_write(p, host(2));
    audio.host_write(p, host(9));
    audio.host_write(p, host(7));
    assert_eq!(watch.poll(), Some(7.0), "the latest of three writes");
    assert_eq!(watch.poll(), None, "and only once");

    audio.host_write(p, host(7));
    assert_eq!(
        watch.poll(),
        None,
        "a write that changes nothing shows nothing"
    );
    assert_eq!(watch.value(), 7.0);
}

#[test]
fn a_host_write_during_a_drag_waits_for_the_close_on_every_surface() {
    let set = set_of(&["p"]);
    let ui = ui_of(&set);
    let audio = set.audio();
    let p = set.id("p").expect("declared");
    let mut dragged = ui.watch(p);
    let mut other = ui.watch(p);

    let drag = ui.begin(p);
    assert!(drag.write(6.0));
    assert_eq!(dragged.poll(), Some(6.0));
    assert_eq!(other.poll(), Some(6.0), "every surface follows the drag");

    audio.host_write(p, host(9));
    assert_eq!(set.value(p), 9.0, "the store takes the host write at once");
    assert_eq!(dragged.poll(), None);
    assert_eq!(other.poll(), None);

    assert!(drag.write(7.0));
    assert_eq!(dragged.poll(), Some(7.0));
    audio.host_write(p, host(3));
    assert_eq!(dragged.poll(), None);
    drop(drag);

    assert_eq!(
        dragged.poll(),
        Some(3.0),
        "the first poll after the close shows the store"
    );
    assert_eq!(other.poll(), Some(3.0));
}

#[test]
fn a_learned_write_mid_touch_moves_the_dsp_but_waits_for_the_watch() {
    let set = set_of(&["p"]);
    let ui = ui_of(&set);
    let audio = set.audio();
    let p = set.id("p").expect("declared");
    let mut watch = ui.watch(p);

    // A touch held still is a gesture too.
    let touch = ui.begin(p);
    audio.learned_write(p, host(8));
    assert_eq!(audio.value(p), 8.0, "the DSP has it");
    assert_eq!(watch.poll(), None, "the surface waits");
    drop(touch);
    assert_eq!(watch.poll(), Some(8.0));
}

#[test]
fn restores_and_syncs_reach_every_watch_and_wait_for_a_gesture() {
    let set = set_of(&["p"]);
    let ui = ui_of(&set);
    let program = set.program();
    let p = set.id("p").expect("declared");
    let mut first = ui.watch(p);
    let mut second = ui.watch(p);

    program.load().restore(p, 2.0);
    assert_eq!(first.poll(), Some(2.0));
    assert_eq!(second.poll(), Some(2.0));
    assert!(program.sync(p, 4.0));
    assert_eq!(first.poll(), Some(4.0));

    let touch = ui.begin(p);
    assert!(program.sync(p, 6.0));
    program.load().restore(p, 8.0);
    assert_eq!(first.poll(), None);
    assert_eq!(
        second.poll(),
        Some(4.0),
        "the value at the touch, written before it"
    );
    drop(touch);
    assert_eq!(first.poll(), Some(8.0));
    assert_eq!(second.poll(), Some(8.0));
}

#[test]
fn a_watch_made_mid_drag_shows_what_the_others_show() {
    // A widget built mid-drag is seeded from what the drag shows, not the
    // store, so it never shows a host write its siblings are holding back.
    let set = set_of(&["p"]);
    let ui = ui_of(&set);
    let audio = set.audio();
    let p = set.id("p").expect("declared");
    let mut early = ui.watch(p);

    let drag = ui.begin(p);
    assert!(drag.write(6.0));
    audio.host_write(p, host(9));
    let mut late = ui.watch(p);
    assert_eq!(late.value(), 6.0, "it seeds from the drag, not the store");
    assert_eq!(early.poll(), Some(6.0));
    assert_eq!(late.poll(), None);
    assert!(drag.write(4.0));
    assert_eq!(late.poll(), Some(4.0));
    assert_eq!(early.poll(), Some(4.0));
    drop(drag);
    assert_eq!(late.poll(), None, "the user's write is the last one");
    assert_eq!(early.poll(), None);

    let drag = ui.begin(p);
    audio.host_write(p, host(1));
    let mut later = ui.watch(p);
    assert_eq!(later.value(), early.value());
    drop(drag);
    assert_eq!(
        later.poll(),
        Some(1.0),
        "the host write it never saw reaches it after the close"
    );
    assert_eq!(early.poll(), Some(1.0));
}

#[test]
fn a_second_token_opened_mid_drag_does_not_show_a_held_host_write() {
    // The gesture state is per parameter, not per token, since tokens nest,
    // so a value box opened during a drag joins what the drag shows.
    let set = set_of(&["p"]);
    let ui = ui_of(&set);
    let audio = set.audio();
    let p = set.id("p").expect("declared");
    let mut watch = ui.watch(p);

    let drag = ui.begin(p);
    assert!(drag.write(6.0));
    audio.host_write(p, host(9));
    let value_box = ui.begin(p);
    assert_eq!(watch.poll(), Some(6.0));
    assert_eq!(watch.poll(), None);
    let mut edit = ui.edit();
    assert!(edit.write(p, 7.0));
    assert_eq!(watch.poll(), Some(7.0));
    audio.host_write(p, host(2));
    drop(edit);
    assert_eq!(watch.poll(), None, "the drag still holds it");
    drop(value_box);
    assert_eq!(watch.poll(), None);
    drop(drag);
    assert_eq!(watch.poll(), Some(2.0));
}

#[test]
fn an_open_program_edit_holds_back_a_host_write() {
    let set = set_of(&["p"]);
    let ui = ui_of(&set);
    let audio = set.audio();
    let p = set.id("p").expect("declared");
    let mut watch = ui.watch(p);

    let mut edit = ui.edit();
    assert!(edit.write(p, 3.0));
    assert_eq!(watch.poll(), Some(3.0), "the edit shows at once");
    audio.host_write(p, host(9));
    assert_eq!(watch.poll(), None, "held while the edit is open");
    drop(edit);
    assert_eq!(watch.poll(), Some(9.0));
}

#[test]
fn two_watches_on_one_parameter_one_dropped_the_survivor_follows() {
    let set = set_of(&["p"]);
    let ui = ui_of(&set);
    let audio = set.audio();
    let program = set.program();
    let p = set.id("p").expect("declared");

    let first = ui.watch(p);
    let mut survivor = ui.watch(p);
    let last = ui.watch(p);
    drop(first);
    drop(last);

    audio.host_write(p, host(1));
    assert_eq!(survivor.poll(), Some(1.0));
    audio.learned_write(p, host(2));
    assert_eq!(survivor.poll(), Some(2.0));
    program.load().restore(p, 3.0);
    assert_eq!(survivor.poll(), Some(3.0));
    assert!(ui.write(p, 4.0));
    assert_eq!(survivor.poll(), Some(4.0));
}

#[test]
fn a_read_never_has_a_count_newer_than_its_value_and_a_poll_never_goes_back() {
    // Write k stores k, then moves the count to k. A read that loaded the
    // value before the count could see a count newer than its value, and a
    // watch keeping that count would never read the newer value. Seeds and
    // polls both read through it. A poll never shows a value older than one
    // it showed.
    const WRITES: u32 = 1_000_000;
    let mut builder = ParamSetBuilder::new();
    builder
        .add(ParamDecl::new(
            "p",
            "p",
            Kind::Continuous(Range::new(0.0, WRITES as f32, 1.0, None).expect("a valid range")),
            0.0,
        ))
        .expect("valid");
    let set = builder.build();
    let ui = ui_of(&set);
    let audio = set.audio();
    let p = set.id("p").expect("declared");
    let mut watch = ui.watch(p);
    let started = Latch::new(1);

    std::thread::scope(|scope| {
        scope.spawn(|| {
            started.count_down();
            for k in 1..=WRITES {
                audio.host_write(p, f64::from(k) / f64::from(WRITES));
            }
        });
        started.wait();
        loop {
            let Reading {
                changes: count,
                value,
                ..
            } = set.read(p);
            assert!(value as u64 >= count, "count {count}, value {value}");
            let shown = watch.value();
            if let Some(next) = watch.poll() {
                assert!(next > shown, "{next} after {shown}");
            }
            if count == u64::from(WRITES) {
                break;
            }
        }
    });

    watch.poll();
    assert_eq!(watch.value(), WRITES as f32);
}

#[test]
fn two_writers_racing_leave_the_watch_on_the_store() {
    // A host thread and the UI write as fast as they can while the UI
    // polls; whatever order they landed in, the watch ends on the store.
    let set = set_of(&["p"]);
    let ui = ui_of(&set);
    let audio = set.audio();
    let p = set.id("p").expect("declared");
    let mut watch = ui.watch(p);
    let started = Latch::new(1);

    std::thread::scope(|scope| {
        scope.spawn(|| {
            started.count_down();
            for i in 0..200_000_u32 {
                audio.host_write(p, host((i % 11) as u8));
            }
        });
        started.wait();
        for i in 0..200_000_u32 {
            ui.write(p, (i % 7) as f32);
            watch.poll();
        }
    });

    watch.poll();
    assert_eq!(watch.value(), set.value(p));
}

#[test]
fn watches_made_while_a_host_thread_writes_end_on_the_store() {
    // The end state under real threads. The ordering itself is
    // pinned by the read test above, which fails where this one, rescued
    // by later writes, would pass.
    let set = set_of(&["p"]);
    let ui = ui_of(&set);
    let audio = set.audio();
    let p = set.id("p").expect("declared");
    let started = Latch::new(1);
    let mut watches = Vec::new();

    std::thread::scope(|scope| {
        scope.spawn(|| {
            started.count_down();
            for i in 0..200_000_u32 {
                audio.host_write(p, host((i % 11) as u8));
            }
            audio.host_write(p, host(10));
        });
        started.wait();
        for _ in 0..20_000 {
            watches.push(ui.watch(p));
        }
    });

    let stored = set.value(p);
    assert_eq!(stored, 10.0);
    for watch in &mut watches {
        watch.poll();
        assert_eq!(watch.value(), stored);
    }
}

/// One step of the UI thread, or a write from another.
#[derive(Debug, Clone)]
enum Op {
    Watch { param: usize },
    Unwatch { watch: usize },
    Poll { watch: usize },
    PollAll,
    Host { param: usize, value: u8 },
    Learned { param: usize, value: u8 },
    Sync { param: usize, value: u8 },
    Restore { param: usize, value: u8 },
    Open { param: usize },
    Close { token: usize },
    Write { token: usize, value: u8 },
    Lone { param: usize, value: u8 },
    Replay { param: usize, value: u8 },
    EditOpen,
    EditWrite { param: usize, value: u8 },
    EditClose,
}

fn op() -> impl Strategy<Value = Op> {
    let value = 0_u8..=10;
    prop_oneof![
        (0_usize..2).prop_map(|param| Op::Watch { param }),
        (0_usize..6).prop_map(|watch| Op::Unwatch { watch }),
        (0_usize..6).prop_map(|watch| Op::Poll { watch }),
        (0_usize..6).prop_map(|watch| Op::Poll { watch }),
        Just(Op::PollAll),
        (0_usize..2, value.clone()).prop_map(|(param, value)| Op::Host { param, value }),
        (0_usize..2, value.clone()).prop_map(|(param, value)| Op::Learned { param, value }),
        (0_usize..2, value.clone()).prop_map(|(param, value)| Op::Sync { param, value }),
        (0_usize..2, value.clone()).prop_map(|(param, value)| Op::Restore { param, value }),
        (0_usize..2).prop_map(|param| Op::Open { param }),
        (0_usize..4).prop_map(|token| Op::Close { token }),
        (0_usize..4, value.clone()).prop_map(|(token, value)| Op::Write { token, value }),
        (0_usize..2, value.clone()).prop_map(|(param, value)| Op::Lone { param, value }),
        (0_usize..2, value.clone()).prop_map(|(param, value)| Op::Replay { param, value }),
        Just(Op::EditOpen),
        (0_usize..2, value).prop_map(|(param, value)| Op::EditWrite { param, value }),
        Just(Op::EditClose),
    ]
}

/// What one parameter's surfaces should show, kept apart from the crate's own
/// gesture state.
#[derive(Debug, Clone, Copy)]
struct Model {
    store: f32,
    open: u32,
    shown: f32,
}

impl Model {
    fn expected(&self) -> f32 {
        if self.open > 0 {
            self.shown
        } else {
            self.store
        }
    }

    fn open(&mut self) {
        if self.open == 0 {
            self.shown = self.store;
        }
        self.open += 1;
    }

    /// Whether a UI-side write of `value` changes what it compares with.
    fn changes(&self, value: f32) -> bool {
        value != self.expected()
    }

    /// A UI-side write: it moves the store and what surfaces show.
    fn ui_write(&mut self, value: f32) {
        if self.changes(value) {
            self.store = value;
            self.shown = value;
        }
    }
}

/// Polls `watch` and checks it shows what the model says, reported once and
/// only on a change.
fn poll_checked(watch: &mut Watch, expected: f32) -> Result<(), TestCaseError> {
    let before = watch.value();
    let polled = watch.poll();
    prop_assert_eq!(watch.value().to_bits(), expected.to_bits());
    prop_assert_eq!(
        polled.map(f32::to_bits),
        (expected.to_bits() != before.to_bits()).then_some(expected.to_bits())
    );
    prop_assert_eq!(watch.poll(), None, "an idle poll reports nothing");
    Ok(())
}

proptest! {
    #![proptest_config(ProptestConfig {
        failure_persistence: Some(Box::new(FileFailurePersistence::Direct(
            "tests/watch.proptest-regressions",
        ))),
        ..ProptestConfig::default()
    })]

    /// With watches made and dropped at random among every kind of write and
    /// gesture, every watch left shows what the model says, so one surface's
    /// drop never stops another following.
    #[test]
    fn every_watch_shows_what_the_model_says(
        ops in proptest::collection::vec(op(), 0..100),
    ) {
        let set = set_of(&["a", "b"]);
        let ui = set.take_ui().expect("the first UI handle");
        let audio = set.audio();
        let program = set.program();
        let ids: Vec<ParamId> = set.ids().collect();
        let mut model = [Model { store: 5.0, open: 0, shown: 5.0 }; 2];
        let mut watches: Vec<(usize, Watch)> = Vec::new();
        let mut tokens: Vec<(usize, Gesture)> = Vec::new();
        let mut edit: Option<(Edit, [bool; 2])> = None;
        for op in &ops {
            match *op {
                Op::Watch { param } => watches.push((param, ui.watch(ids[param]))),
                Op::Unwatch { watch } => {
                    if watch < watches.len() {
                        drop(watches.remove(watch));
                    }
                }
                Op::Poll { watch } => {
                    if let Some((param, watch)) = watches.get_mut(watch) {
                        poll_checked(watch, model[*param].expected())?;
                    }
                }
                Op::PollAll => {
                    for (param, watch) in &mut watches {
                        poll_checked(watch, model[*param].expected())?;
                    }
                }
                Op::Host { param, value } => {
                    audio.host_write(ids[param], host(value));
                    model[param].store = f32::from(value);
                }
                Op::Learned { param, value } => {
                    audio.learned_write(ids[param], host(value));
                    model[param].store = f32::from(value);
                }
                Op::Sync { param, value } => {
                    program.sync(ids[param], f32::from(value));
                    model[param].store = f32::from(value);
                }
                Op::Restore { param, value } => {
                    program.load().restore(ids[param], f32::from(value));
                    model[param].store = f32::from(value);
                }
                Op::Open { param } => {
                    tokens.push((param, ui.begin(ids[param])));
                    model[param].open();
                }
                Op::Close { token } => {
                    if token < tokens.len() {
                        let (param, gesture) = tokens.remove(token);
                        drop(gesture);
                        model[param].open -= 1;
                    }
                }
                Op::Write { token, value } => {
                    if let Some((param, gesture)) = tokens.get(token) {
                        gesture.write(f32::from(value));
                        model[*param].ui_write(f32::from(value));
                    }
                }
                Op::Lone { param, value } => {
                    ui.write(ids[param], f32::from(value));
                    model[param].ui_write(f32::from(value));
                }
                Op::Replay { param, value } => {
                    ui.replay(ids[param], f32::from(value));
                    model[param].ui_write(f32::from(value));
                }
                Op::EditOpen => {
                    if edit.is_none() {
                        edit = Some((ui.edit(), [false; 2]));
                    }
                }
                Op::EditWrite { param, value } => {
                    if let Some((edit, opened)) = edit.as_mut() {
                        // An edit opens its gesture on a parameter at its
                        // first changing write.
                        let model = &mut model[param];
                        if model.changes(f32::from(value)) && !opened[param] {
                            opened[param] = true;
                            model.open();
                        }
                        model.ui_write(f32::from(value));
                        edit.write(ids[param], f32::from(value));
                    }
                }
                Op::EditClose => {
                    if let Some((edit, opened)) = edit.take() {
                        drop(edit);
                        for (param, was) in opened.into_iter().enumerate() {
                            if was {
                                model[param].open -= 1;
                            }
                        }
                    }
                }
            }
        }
        tokens.clear();
        drop(edit);
        for (param, watch) in &mut watches {
            poll_checked(watch, model[*param].store)?;
            prop_assert_eq!(watch.value(), set.value(ids[*param]));
        }
    }
}
