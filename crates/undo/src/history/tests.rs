use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use seam_params::{Kind, ParamDecl, ParamSet, ParamSetBuilder, UiParams};

use super::*;

fn ui() -> (Arc<ParamSet>, UiParams) {
    let mut builder = ParamSetBuilder::new();
    builder
        .add(ParamDecl::new("p", "p", Kind::Bool, 0.0))
        .expect("valid");
    let set = builder.build();
    let ui = set.take_ui().expect("UI");
    (set, ui)
}

fn history(bound: Bound) -> (Arc<ParamSet>, UiParams, History) {
    let (set, ui) = ui();
    let history = History::new(&ui, bound).expect("the first history");
    (set, ui, history)
}

/// A step that logs its replays and may call back into a history.
#[derive(Debug)]
struct Probe {
    name: &'static str,
    log: Rc<RefCell<Vec<String>>>,
    units: usize,
    nested: Option<History>,
    panics: bool,
}

impl Probe {
    fn boxed(name: &'static str, log: &Rc<RefCell<Vec<String>>>) -> Vec<Box<dyn Step>> {
        vec![Box::new(Probe {
            name,
            log: Rc::clone(log),
            units: booked(2 * size_of::<f32>()),
            nested: None,
            panics: false,
        })]
    }

    fn replayed(&mut self, verb: &str) {
        if self.panics {
            panic!("a replay failed");
        }
        let mut line = format!("{verb} {}", self.name);
        if let Some(history) = &self.nested {
            let nested = (
                history.undo(),
                history.redo(),
                history.jump(0),
                history.clear(),
                history.record("nested", Vec::new()),
            );
            line += &format!(" {nested:?}");
        }
        self.log.borrow_mut().push(line);
    }
}

impl Step for Probe {
    fn undo(&mut self) {
        self.replayed("undo");
    }
    fn redo(&mut self) {
        self.replayed("redo");
    }
    fn units(&self) -> usize {
        self.units
    }
}

fn names(history: &History) -> Vec<(String, bool)> {
    history
        .entries()
        .into_iter()
        .map(|entry| (entry.name, entry.undone))
        .collect()
}

#[test]
fn undo_and_redo_move_the_cursor_and_a_new_entry_drops_the_redo() {
    let (_set, _ui, history) = history(Bound::default());
    let log = Rc::new(RefCell::new(Vec::new()));
    history.record("a", Probe::boxed("a", &log)).expect("idle");
    history.record("b", Probe::boxed("b", &log)).expect("idle");
    assert_eq!(history.cursor(), 2);
    assert_eq!(history.undo(), Ok(true));
    assert_eq!(names(&history), [("a".into(), false), ("b".into(), true)]);
    assert_eq!(history.redo(), Ok(true));
    assert_eq!(history.redo(), Ok(false));
    assert_eq!(history.undo(), Ok(true));
    history.record("c", Probe::boxed("c", &log)).expect("idle");
    assert_eq!(names(&history), [("a".into(), false), ("c".into(), false)]);
    assert_eq!(*log.borrow(), ["undo b", "redo b", "undo b"]);
    assert_eq!(history.undo(), Ok(true));
    assert_eq!(history.undo(), Ok(true));
    assert_eq!(history.undo(), Ok(false));
}

#[test]
fn a_request_made_during_a_replay_is_refused_and_the_cursor_moves_once() {
    let (_set, _ui, history) = history(Bound::default());
    let log = Rc::new(RefCell::new(Vec::new()));
    history.record("a", Probe::boxed("a", &log)).expect("idle");
    let nested = Probe {
        name: "b",
        log: Rc::clone(&log),
        units: booked(2 * size_of::<f32>()),
        nested: Some(history.clone()),
        panics: false,
    };
    history.record("b", vec![Box::new(nested)]).expect("idle");

    assert_eq!(history.undo(), Ok(true));
    assert_eq!(history.cursor(), 1);
    assert_eq!(history.redo(), Ok(true));
    assert_eq!(history.cursor(), 2);
    let refused = format!(
        "{:?}",
        (
            Err::<bool, _>(HistoryError::Replaying),
            Err::<bool, _>(HistoryError::Replaying),
            Err::<(), _>(HistoryError::Replaying),
            Err::<(), _>(HistoryError::Replaying),
            Err::<(), _>(HistoryError::Replaying),
        )
    );
    assert_eq!(
        *log.borrow(),
        [format!("undo b {refused}"), format!("redo b {refused}")]
    );
    assert_eq!(names(&history), [("a".into(), false), ("b".into(), false)]);
}

#[test]
fn undo_waits_while_a_user_gesture_is_open() {
    let (set, ui, history) = history(Bound::default());
    let log = Rc::new(RefCell::new(Vec::new()));
    history.record("a", Probe::boxed("a", &log)).expect("idle");
    let p = set.id("p").expect("declared");
    let touch = ui.begin(p);
    assert_eq!(history.undo(), Err(HistoryError::GestureOpen));
    assert_eq!(history.redo(), Err(HistoryError::GestureOpen));
    assert_eq!(history.jump(0), Err(HistoryError::GestureOpen));
    assert_eq!(history.cursor(), 1);
    drop(touch);
    assert_eq!(history.undo(), Ok(true));
    assert!(log.borrow().len() == 1);
}

#[test]
fn a_jump_hops_one_entry_at_a_time_and_is_one_change() {
    let (_set, _ui, history) = history(Bound::default());
    let log = Rc::new(RefCell::new(Vec::new()));
    for name in ["a", "b", "c"] {
        history
            .record(name, Probe::boxed(name, &log))
            .expect("idle");
    }
    let version = history.version();
    assert_eq!(history.jump(0), Ok(true));
    assert_eq!(history.version(), version + 1);
    assert_eq!(
        names(&history),
        [("a".into(), true), ("b".into(), true), ("c".into(), true)],
        "the undone entries list in redo order"
    );
    assert_eq!(history.jump(2), Ok(true));
    assert_eq!(history.cursor(), 2);
    assert_eq!(
        *log.borrow(),
        ["undo c", "undo b", "undo a", "redo a", "redo b"]
    );
    let version = history.version();
    assert_eq!(history.jump(2), Ok(false), "already there");
    assert_eq!(history.version(), version, "no visible change");
    assert_eq!(
        history.jump(4),
        Err(HistoryError::OutOfRange { index: 4, len: 3 })
    );
    assert_eq!(history.cursor(), 2);
    assert_eq!(history.version(), version);
}

#[test]
fn the_listing_counter_moves_only_on_a_visible_change() {
    let (_set, _ui, history) = history(Bound::default());
    let log = Rc::new(RefCell::new(Vec::new()));
    let start = history.version();
    assert_eq!(history.undo(), Ok(false));
    assert_eq!(history.redo(), Ok(false));
    history.clear().expect("idle");
    history.record("empty", Vec::new()).expect("idle");
    assert_eq!(history.version(), start);
    history.record("a", Probe::boxed("a", &log)).expect("idle");
    assert_eq!(history.version(), start + 1);
    history.clear().expect("idle");
    assert_eq!(history.version(), start + 2);
    assert!(history.is_empty());
}

#[test]
fn undo_and_redo_each_move_the_listing_counter() {
    let (_set, _ui, history) = history(Bound::default());
    let log = Rc::new(RefCell::new(Vec::new()));
    history.record("a", Probe::boxed("a", &log)).expect("idle");
    let version = history.version();
    assert_eq!(history.undo(), Ok(true));
    assert_eq!(history.version(), version + 1);
    assert_eq!(history.redo(), Ok(true));
    assert_eq!(history.version(), version + 2);
}

#[test]
fn a_step_joining_an_entry_moves_the_counter_only_when_pruning_drops_one() {
    // The joined step adds its units, so it can prune.
    let (_set, _ui, history) = history(Bound::default());
    let log = Rc::new(RefCell::new(Vec::new()));
    let transaction = history.transaction("t");
    history.record("a", Probe::boxed("a", &log)).expect("idle");
    let (listing, version) = (names(&history), history.version());
    history.record("b", Probe::boxed("b", &log)).expect("idle");
    assert_eq!(names(&history), listing);
    assert_eq!(history.version(), version, "the listing reads the same");
    drop(transaction);

    let bound = Bound {
        max_units: 100,
        min_entries: 1,
    };
    let (_set, _ui, history) = self::history(bound);
    history.record("x", Probe::boxed("x", &log)).expect("idle");
    let transaction = history.transaction("t");
    history.record("a", Probe::boxed("a", &log)).expect("idle");
    let version = history.version();
    history.record("b", Probe::boxed("b", &log)).expect("idle");
    assert_eq!(names(&history), [("t".into(), false)], "x pruned");
    assert_eq!(history.version(), version + 1);
    drop(transaction);
}

#[test]
fn a_scope_whose_entry_is_pruned_away_starts_a_new_one() {
    // The second step pushes the scope's entry over the cap with no floor,
    // so it goes whole, and the next step cannot join what is gone.
    let bound = Bound {
        max_units: 50,
        min_entries: 0,
    };
    let (_set, _ui, history) = history(bound);
    let log = Rc::new(RefCell::new(Vec::new()));
    let _transaction = history.transaction("t");
    history.record("a", Probe::boxed("a", &log)).expect("idle");
    history.record("b", Probe::boxed("b", &log)).expect("idle");
    assert!(history.is_empty(), "pruned whole");
    history.record("c", Probe::boxed("c", &log)).expect("idle");
    assert_eq!(names(&history), [("t".into(), false)]);
}

#[test]
fn a_dropped_redo_gives_back_its_units() {
    // 40 + 40 units fit a cap of 100 once the redo is gone.
    let bound = Bound {
        max_units: 100,
        min_entries: 1,
    };
    let (_set, _ui, history) = history(bound);
    let log = Rc::new(RefCell::new(Vec::new()));
    history.record("a", Probe::boxed("a", &log)).expect("idle");
    history.record("b", Probe::boxed("b", &log)).expect("idle");
    assert_eq!(history.undo(), Ok(true));
    history.record("c", Probe::boxed("c", &log)).expect("idle");
    assert_eq!(names(&history), [("a".into(), false), ("c".into(), false)]);
}

#[test]
fn a_step_books_its_values_plus_32_once_and_pruning_drops_whole_entries() {
    // A parameter step stores two f32s, so books 40 units; 30,000 units
    // hold 750 of them.
    assert_eq!(booked(2 * size_of::<f32>()), 40);
    let (_set, _ui, history) = history(Bound::default());
    let log = Rc::new(RefCell::new(Vec::new()));
    for _ in 0..800 {
        history.record("p", Probe::boxed("p", &log)).expect("idle");
    }
    assert_eq!(history.len(), 750);
    assert_eq!(history.cursor(), 750);

    // The floor counts entries: two-step entries over a tiny cap keep
    // exactly the floor, each entry whole.
    let bound = Bound {
        max_units: 100,
        min_entries: 3,
    };
    let (_set, _ui, history) = self::history(bound);
    for _ in 0..10 {
        let mut steps = Probe::boxed("x", &log);
        steps.extend(Probe::boxed("y", &log));
        history.record("xy", steps).expect("idle");
    }
    assert_eq!(history.len(), 3);
    log.borrow_mut().clear();
    history.jump(0).expect("idle");
    assert_eq!(log.borrow().len(), 6, "every pruned entry went whole");
}

/// A step whose replay stores a restore, as a load on another thread
/// might mid-walk.
#[derive(Debug)]
struct Loads(seam_params::RestoreWriter);

impl Step for Loads {
    fn undo(&mut self) {
        self.0.stored();
    }
    fn redo(&mut self) {
        self.0.stored();
    }
    fn units(&self) -> usize {
        STEP_OVERHEAD
    }
}

#[test]
fn a_replay_stops_before_its_next_write_when_a_restore_is_stored() {
    // A restore stored during a walk stops it before its next step and
    // clears the history when the walk ends.
    let (set, ui) = ui();
    let clock = Arc::clone(set.clock());
    let history = History::new(&ui, Bound::default()).expect("the first history");
    let log = Rc::new(RefCell::new(Vec::new()));
    let mut steps = Probe::boxed("first", &log);
    steps.push(Box::new(Loads(clock.writer())));
    steps.extend(Probe::boxed("last", &log));
    history.record("entry", steps).expect("idle");
    history
        .record("next", Probe::boxed("next", &log))
        .expect("idle");

    assert_eq!(history.jump(0), Ok(false), "a restore stopped it");
    assert_eq!(*log.borrow(), ["undo next", "undo last"]);
    assert!(history.is_empty());
    assert_eq!(history.cursor(), 0);
    history
        .record("after", Probe::boxed("after", &log))
        .expect("idle");
    assert_eq!(history.len(), 1);

    // An undo a restore stopped says it did not undo the entry.
    log.borrow_mut().clear();
    let mut steps = Probe::boxed("first", &log);
    steps.push(Box::new(Loads(clock.writer())));
    steps.extend(Probe::boxed("last", &log));
    history.record("entry", steps).expect("idle");
    let version = history.version();
    assert_eq!(history.undo(), Ok(false));
    assert_eq!(*log.borrow(), ["undo last"]);
    assert!(history.is_empty());
    assert_eq!(history.version(), version + 1, "moved once, by the clear");
}

/// A step that, while it replays, starts a thread that takes the load
/// hold, and notes whether that thread got it before the step ended.
#[derive(Debug)]
struct Loading {
    clock: seam_params::RestoreReader,
    parameters: bool,
    took: Rc<RefCell<Vec<bool>>>,
    threads: Rc<RefCell<Vec<std::thread::JoinHandle<()>>>>,
}

impl Loading {
    fn write(&mut self) {
        let took = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let (clock, flag) = (self.clock.clone(), Arc::clone(&took));
        self.threads.borrow_mut().push(std::thread::spawn(move || {
            let _held = clock.hold_loads();
            flag.store(true, std::sync::atomic::Ordering::SeqCst);
        }));
        // Held, it cannot take the hold however long it gets; free, it
        // gets five seconds, so a loaded machine cannot fail the test.
        let wait = if self.parameters { 30 } else { 5_000 };
        let start = std::time::Instant::now();
        while !took.load(std::sync::atomic::Ordering::SeqCst)
            && start.elapsed() < std::time::Duration::from_millis(wait)
        {
            std::thread::yield_now();
        }
        self.took
            .borrow_mut()
            .push(took.load(std::sync::atomic::Ordering::SeqCst));
    }
}

impl Step for Loading {
    fn undo(&mut self) {
        self.write();
    }
    fn redo(&mut self) {
        self.write();
    }
    fn units(&self) -> usize {
        STEP_OVERHEAD
    }
    fn writes_parameters(&self) -> bool {
        self.parameters
    }
}

#[test]
fn a_step_that_writes_parameters_holds_loads_off_and_no_other_does() {
    // The hold covers a parameter write, undo and redo alike, and
    // nothing else, so it stays a leaf lock.
    let (set, ui) = ui();
    let clock = Arc::clone(set.clock());
    let history = History::new(&ui, Bound::default()).expect("the first history");
    let took = Rc::new(RefCell::new(Vec::new()));
    let threads = Rc::new(RefCell::new(Vec::new()));
    for parameters in [true, false] {
        let step = Loading {
            clock: clock.reader(),
            parameters,
            took: Rc::clone(&took),
            threads: Rc::clone(&threads),
        };
        history.record("step", vec![Box::new(step)]).expect("idle");
        assert_eq!(history.undo(), Ok(true));
        assert_eq!(history.redo(), Ok(true));
        for thread in threads.borrow_mut().drain(..) {
            thread.join().expect("the loading thread");
        }
        let expected = [!parameters, !parameters];
        assert_eq!(*took.borrow(), expected, "parameters: {parameters}");
        took.borrow_mut().clear();
    }
}

#[test]
fn a_replay_that_panics_leaves_an_empty_history() {
    let (_set, _ui, history) = history(Bound::default());
    let log = Rc::new(RefCell::new(Vec::new()));
    history.record("a", Probe::boxed("a", &log)).expect("idle");
    let failing = Probe {
        name: "b",
        log: Rc::clone(&log),
        units: booked(2 * size_of::<f32>()),
        nested: None,
        panics: true,
    };
    history.record("b", vec![Box::new(failing)]).expect("idle");
    let version = history.version();
    let unwound = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| history.undo()));
    assert!(unwound.is_err());
    assert!(history.is_empty());
    assert_eq!(history.cursor(), 0);
    assert!(history.version() > version);
    history
        .record("c", Probe::boxed("c", &log))
        .expect("idle again");
    assert_eq!(history.undo(), Ok(true));
}
