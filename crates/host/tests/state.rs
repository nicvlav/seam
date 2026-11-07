//! The saved block: its JSON shape, which blocks load, and how a load meets
//! the session and other threads.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::Arc;

use seam_host::{LoadError, MAX_STATE_BYTES, SavedState};
use seam_params::{Kind, ParamDecl, ParamId, ParamSet, ParamSetBuilder, RestoreReader};
use seam_session::{ApplyError, Participant, Section, Session};
use serde_json::json;

const MODES: &[&str] = &["low", "band", "high"];

fn decl(key: &'static str, group: &'static str, kind: Kind, default: f32) -> ParamDecl {
    ParamDecl::new(key, key, kind, default).with_group(group)
}

fn range(start: f32, end: f32, interval: f32, skew_centre: Option<f32>) -> Kind {
    Kind::continuous(start, end, interval, skew_centre)
}

/// Declared out of host order on purpose: `mode` sits in a later group.
fn set() -> Arc<ParamSet> {
    ParamSet::from_decls([
        decl(
            "freq",
            "filter",
            range(20.0, 20_000.0, 0.0, Some(1_000.0)),
            1_000.0,
        ),
        decl("gain", "", range(-24.0, 24.0, 0.0, None), 0.0),
        decl("on", "", Kind::Bool, 1.0),
        decl("mode", "filter", Kind::Choice(MODES), 1.0),
        decl("steps", "", range(0.0, 10.0, 0.5, None), 2.0),
    ])
    .expect("valid")
}

struct Rig {
    set: Arc<ParamSet>,
    session: Session,
    clock: RestoreReader,
    state: SavedState,
}

fn rig() -> Rig {
    rig_of(set())
}

fn rig_of(set: Arc<ParamSet>) -> Rig {
    let session = Session::new(&set);
    let state = SavedState::new(&set, &session);
    let clock = set.clock().reader();
    Rig {
        set,
        session,
        clock,
        state,
    }
}

impl Rig {
    fn id(&self, key: &str) -> ParamId {
        self.set.id(key).expect("declared")
    }

    fn write(&self, key: &str, value: f32) {
        let load = self.set.program().load();
        load.restore(self.id(key), value);
    }

    fn saved(&self) -> serde_json::Value {
        serde_json::from_slice(&self.state.save()).expect("JSON")
    }

    fn changes(&self) -> u64 {
        self.set.ids().map(|id| self.set.changes(id)).sum()
    }
}

#[test]
fn a_block_is_version_parameters_extra_in_that_order_with_each_value_typed_by_kind() {
    // A bool is a bool, a choice an integer, the rest a double that holds
    // the f32 exactly. Parameters come in host order, and an empty `extra`
    // is left out.
    let rig = rig();
    rig.write("gain", 0.1);
    rig.write("mode", 2.0);
    rig.write("on", 0.0);
    let text = String::from_utf8(rig.state.save()).expect("UTF-8");
    let gain = f64::from(0.1_f32);
    assert_eq!(
        text,
        format!(
            "{{\"seamStateVersion\":1,\"parameters\":{{\"gain\":{gain},\"on\":false,\"steps\":2.0,\
             \"freq\":1000.0,\"mode\":2}}}}"
        )
    );
    assert!(!text.starts_with('\u{feff}'));

    let block = rig.saved();
    assert!(
        block["parameters"]["mode"].is_u64(),
        "a choice is an integer"
    );
    assert_eq!(
        block["parameters"]["gain"].as_f64().map(|v| v as f32),
        Some(0.1)
    );
}

#[test]
fn save_load_save_gives_the_same_bytes_on_linear_skewed_and_stepped_ranges() {
    // A value that saved once saves the same bytes after a load.
    let rig = rig();
    rig.write("freq", 1234.567);
    rig.write("gain", -3.3);
    rig.write("steps", 7.5);
    let saved = rig.state.save();
    let other = self::rig();
    assert!(other.state.load(&saved).is_ok());
    assert_eq!(other.state.save(), saved);
    for key in ["freq", "gain", "steps"] {
        assert_eq!(
            other.set.value(other.id(key)).to_bits(),
            rig.set.value(rig.id(key)).to_bits(),
            "{key}"
        );
    }
}

#[test]
fn a_block_that_is_not_recognised_changes_nothing_at_all() {
    // Neither the parameters nor the session see a block that is refused.
    let rig = rig();
    rig.session.restore(Some(&json!({"kept": "x"})));
    let before = (rig.changes(), rig.clock.stamp(), rig.session.save());
    let bom = b"\xef\xbb\xbf{\"seamStateVersion\":1,\"parameters\":{}}".to_vec();
    let trailing = b"{\"seamStateVersion\":1,\"parameters\":{}} x".to_vec();
    for block in [
        b"".to_vec(),
        vec![0, 159, 146, 150],
        b"not json".to_vec(),
        b"[1, 2]".to_vec(),
        br#"{"parameters": {}}"#.to_vec(),
        br#"{"seamStateVersion": "1", "parameters": {}}"#.to_vec(),
        br#"{"seamStateVersion": 0, "parameters": {}}"#.to_vec(),
        br#"{"seamStateVersion": 1.5, "parameters": {}}"#.to_vec(),
        br#"{"seamStateVersion": 1, "parameters": []}"#.to_vec(),
        bom,
        trailing,
    ] {
        assert!(rig.state.load(&block).is_err(), "{block:?}");
        assert_eq!(
            (rig.changes(), rig.clock.stamp(), rig.session.save()),
            before,
            "{block:?}"
        );
    }
}

#[test]
fn a_recognised_block_assigns_every_parameter_whatever_was_there_before() {
    // A missing, mistyped or out-of-range value takes the default;
    // anything else snaps; a newer version loads best-effort.
    let rig = rig();
    rig.write("gain", 12.0);
    let block = json!({"seamStateVersion": 7, "newerRoot": true, "parameters": {
        "freq": "loud",
        "on": false,
        "mode": 1.6,
        "steps": 1e39,
        "unknown": [1],
    }});
    assert!(rig.state.load(block.to_string().as_bytes()).is_ok());
    let value = |key| rig.set.value(rig.id(key));
    assert_eq!(value("gain"), 0.0, "missing: the default, not the previous");
    assert_eq!(value("freq"), 1000.0, "mistyped: the default");
    assert_eq!(value("on"), 0.0, "a bool, against a default of on");
    assert_eq!(value("mode"), 2.0, "snapped by the restore write");
    assert_eq!(value("steps"), 2.0, "outside f32: the default");
}

#[test]
fn unknown_keys_are_kept_saved_after_the_declared_and_replaced_by_the_next_load() {
    // A newer build's keys survive a load and a save.
    let rig = rig();
    let block = json!({"seamStateVersion": 1, "parameters": {"newer": 3.5, "gain": 1.0}});
    assert!(rig.state.load(block.to_string().as_bytes()).is_ok());
    let text = String::from_utf8(rig.state.save()).expect("UTF-8");
    assert!(text.contains(",\"mode\":1,\"newer\":3.5}"), "{text}");
    let block = json!({"seamStateVersion": 1, "parameters": {"other": false}});
    assert!(rig.state.load(block.to_string().as_bytes()).is_ok());
    let saved = rig.saved();
    assert_eq!(saved["parameters"].get("newer"), None);
    assert_eq!(saved["parameters"]["other"], false);
}

#[test]
fn unknown_top_level_keys_are_kept_saved_after_extra_in_key_order_and_replaced_by_the_next_load() {
    // A newer build's top-level content survives the version a save keeps.
    let rig = rig();
    let block = br#"{"seamStateVersion":2,"parameters":{"gain":3},"future":{"x":1},
        "extra":{"notes":"n"},"another":[true]}"#;
    assert!(rig.state.load(block).is_ok());
    let text = String::from_utf8(rig.state.save()).expect("UTF-8");
    assert!(
        text.starts_with(r#"{"seamStateVersion":2,"parameters":{"gain":3.0,"#),
        "{text}"
    );
    assert!(
        text.ends_with(r#","extra":{"notes":"n"},"another":[true],"future":{"x":1}}"#),
        "{text}"
    );

    // Saved, loaded and saved again, the block is the same bytes.
    let other = self::rig();
    assert!(other.state.load(text.as_bytes()).is_ok());
    assert_eq!(other.state.save(), text.as_bytes());

    // The next load replaces what was kept, here with nothing.
    let block = json!({"seamStateVersion": 1, "parameters": {}});
    assert!(rig.state.load(block.to_string().as_bytes()).is_ok());
    let saved = rig.saved();
    assert_eq!(
        saved
            .as_object()
            .map(|root| root.keys().map(String::as_str).collect::<Vec<_>>()),
        Some(vec!["parameters", "seamStateVersion"])
    );
}

/// A participant that records the parameter value it saw when it applied.
struct Reads {
    set: Arc<ParamSet>,
    id: ParamId,
    saw: Cell<Option<f32>>,
}

impl Participant for Reads {
    fn capture(&self) -> Section {
        json!("reads")
    }

    fn apply(&mut self, _: Option<&Section>) -> Result<(), ApplyError> {
        self.saw.set(Some(self.set.value(self.id)));
        Ok(())
    }
}

#[test]
fn parameters_are_assigned_before_the_session_applies_extra() {
    // With a real session and a real history on one clock, the load clears
    // undo, and a participant's apply sees the restored values.
    let set = set();
    let session = Session::new(&set);
    let ui = session.take_ui().expect("the first UI handle");
    let state = SavedState::new(&set, &session);
    let params = set.take_ui().expect("the UI handle");
    let history = seam_undo::History::new(&params, seam_undo::Bound::default()).expect("history");
    history.enroll_params();
    let gain = set.id("gain").expect("declared");
    assert!(params.write(gain, 6.0));
    history.tick();
    assert_eq!(history.len(), 1);

    let reads = Rc::new(RefCell::new(Reads {
        set: Arc::clone(&set),
        id: gain,
        saw: Cell::new(None),
    }));
    let _reads = ui.register("reads", &reads).expect("free");
    let block = json!({"seamStateVersion": 1, "parameters": {"gain": -6.0},
                       "extra": {"reads": "x"}});
    assert!(state.load(block.to_string().as_bytes()).is_ok());
    assert!(history.is_empty(), "the load cleared undo");
    assert_eq!(history.undo(), Ok(false));
    assert_eq!(reads.borrow().saw.get(), None, "applied on a later tick");
    ui.tick();
    assert_eq!(reads.borrow().saw.get(), Some(-6.0));
    assert_eq!(set.value(gain), -6.0);
}

#[test]
fn a_save_racing_loads_writes_one_block_or_the_other_never_a_mix() {
    // A host may save and load on two threads at once. The blocks differ in
    // version and in the keys a save keeps too, so a save that paired one
    // load's version with another's keys would show.
    let rig = rig();
    let a = json!({"seamStateVersion": 1, "parameters": {"gain": -6.0, "zzA": 1},
                   "extra": {"tag": 1}, "olderA": 1})
    .to_string();
    let b = json!({"seamStateVersion": 3, "parameters": {"gain": 6.0, "zzB": 2},
                   "extra": {"tag": 2}, "olderB": 2})
    .to_string();
    assert!(rig.state.load(a.as_bytes()).is_ok());
    let saved_a = rig.state.save();
    assert!(rig.state.load(b.as_bytes()).is_ok());
    let saved_b = rig.state.save();
    let state = &rig.state;
    std::thread::scope(|scope| {
        scope.spawn(|| {
            for round in 0..2_000 {
                let block = if round % 2 == 0 { &a } else { &b };
                assert!(state.load(block.as_bytes()).is_ok());
            }
        });
        for _ in 0..2_000 {
            let saved = state.save();
            assert!(saved == saved_a || saved == saved_b, "a mixed block");
        }
    });
}

/// A participant that records what one parameter held when it applied.
struct SeesLast {
    set: Arc<ParamSet>,
    last: ParamId,
    saw: Rc<RefCell<Vec<f32>>>,
}

impl Participant for SeesLast {
    fn capture(&self) -> Section {
        Section::Null
    }

    fn apply(&mut self, _: Option<&Section>) -> Result<(), ApplyError> {
        self.saw.borrow_mut().push(self.set.value(self.last));
        Ok(())
    }
}

#[test]
fn parameters_land_before_a_participant_applies_while_the_ui_thread_ticks() {
    // A long load on a host thread while the UI thread ticks the session
    // throughout never shows a participant an old value.
    let keys: Vec<&'static str> = (0..3_000)
        .map(|index| &*Box::leak(format!("p{index}").into_boxed_str()))
        .collect();
    let set = ParamSet::from_decls(
        keys.iter()
            .map(|&key| decl(key, "", range(0.0, 100.0, 0.0, None), 0.0)),
    )
    .expect("valid");
    let last = set.id(keys[keys.len() - 1]).expect("declared");
    let session = Session::new(&set);
    let ui = session.take_ui().expect("the first UI handle");
    let saw = Rc::new(RefCell::new(Vec::new()));
    let sees = Rc::new(RefCell::new(SeesLast {
        set: Arc::clone(&set),
        last,
        saw: Rc::clone(&saw),
    }));
    let _sees = ui.register("sees", &sees).expect("free");
    let state = SavedState::new(&set, &session);
    for round in 0..20 {
        let value = (round % 2 * 50 + 25) as f32;
        let parameters: serde_json::Map<String, serde_json::Value> =
            keys.iter().map(|&key| (key.into(), json!(value))).collect();
        let block = json!({"seamStateVersion": 1, "parameters": parameters,
                           "extra": {"sees": round}})
        .to_string();
        let done = std::sync::atomic::AtomicBool::new(false);
        std::thread::scope(|scope| {
            scope.spawn(|| {
                assert!(state.load(block.as_bytes()).is_ok());
                done.store(true, std::sync::atomic::Ordering::Release);
            });
            while !done.load(std::sync::atomic::Ordering::Acquire) {
                ui.tick();
            }
        });
        ui.tick();
        for seen in saw.borrow_mut().drain(..) {
            assert_eq!(seen, value, "round {round}: an apply saw an old value");
        }
    }
}

#[test]
fn a_load_with_no_unknown_keys_empties_the_kept_set() {
    // A load replaces the kept set.
    let rig = rig();
    let block = json!({"seamStateVersion": 1, "parameters": {"newer": 1}});
    assert!(rig.state.load(block.to_string().as_bytes()).is_ok());
    let block = json!({"seamStateVersion": 1, "parameters": {"gain": 1}});
    assert!(rig.state.load(block.to_string().as_bytes()).is_ok());
    assert_eq!(rig.saved()["parameters"].get("newer"), None);
}

#[test]
fn a_saved_state_refuses_a_session_made_from_another_set() {
    // Its restores would mark another clock than the load's, and an undo
    // could write a pre-load value over a loaded one.
    let set = set();
    let session = Session::new(&ParamSetBuilder::new().build());
    seam_testkit::contract::expect_contract_panic("made from another ParamSet", || {
        let _ = SavedState::new(&set, &session);
    });
}

#[test]
fn a_load_marks_the_clock_every_history_of_the_set_reads() {
    // The stored count moves on the set's own clock.
    let rig = rig();
    assert!(rig.session.uses(&rig.set));
    let before = rig.clock.stamp().stored();
    let block = json!({"seamStateVersion": 1, "parameters": {"gain": 1}});
    assert!(rig.state.load(block.to_string().as_bytes()).is_ok());
    assert_eq!(
        rig.clock.stamp().stored(),
        before + 2,
        "the load, then the session's store"
    );
}

#[test]
fn a_refused_block_says_which_check_it_failed() {
    let rig = rig();
    let load = |block: &[u8]| rig.state.load(block);
    assert_eq!(load(b"not json"), Err(LoadError::NotJson));
    assert_eq!(
        load(b"{} {}"),
        Err(LoadError::NotJson),
        "bytes after the object"
    );
    assert_eq!(load(b"[1, 2]"), Err(LoadError::NotAnObject));
    assert_eq!(
        load(br#"{"seamStateVersion": 0, "parameters": {}}"#),
        Err(LoadError::BadVersion)
    );
    assert_eq!(
        load(br#"{"seamStateVersion": 1, "parameters": []}"#),
        Err(LoadError::BadParameters)
    );
    assert_eq!(
        load(br#"{"seamStateVersion": 1}"#),
        Err(LoadError::BadParameters)
    );
}

#[test]
fn a_block_past_the_limit_is_refused_unread() {
    let rig = rig();
    let mut block = br#"{"seamStateVersion": 1, "parameters": {"gain": 6}}"#.to_vec();
    assert!(rig.state.load(&block).is_ok());
    block.resize(MAX_STATE_BYTES + 1, b' ');
    rig.write("gain", 1.0);
    assert_eq!(
        rig.state.load(&block),
        Err(LoadError::TooLarge),
        "one byte past the limit"
    );
    assert_eq!(rig.set.value(rig.id("gain")), 1.0, "and nothing changed");
}

#[test]
fn a_number_past_f64s_range_is_refused_alone_and_the_rest_loads() {
    // The JSON parser refuses a whole text for one such number.
    let rig = rig();
    let block = br#"{"seamStateVersion": 1, "parameters": {"gain": 1e400, "freq": 500,
        "steps": -2e999}, "extra": {"kept": {"big": 1e400, "text": "1e400", "small": 1e-400}}}"#;
    assert!(rig.state.load(block).is_ok());
    let value = |key| rig.set.value(rig.id(key));
    assert_eq!(value("gain"), 0.0, "the default");
    assert_eq!(value("steps"), 2.0, "the default");
    assert_eq!(value("freq"), 500.0, "the rest loads");
    assert_eq!(
        rig.saved()["extra"]["kept"],
        json!({"big": null, "text": "1e400", "small": 0.0}),
        "a number in a section too; a string is left alone"
    );
}

#[test]
fn a_number_json_does_not_allow_is_refused_even_past_f64s_range() {
    // Only a number JSON allows reads as null when it is too large; any
    // other token makes the block not JSON, as the parser alone would.
    let rig = rig();
    rig.write("gain", 1.0);
    for number in ["01e400", "1.e400", "-01e400", "1e400.5", ".5e400"] {
        let block = format!(r#"{{"seamStateVersion": 1, "parameters": {{"gain": {number}}}}}"#);
        assert_eq!(
            rig.state.load(block.as_bytes()),
            Err(LoadError::NotJson),
            "{number}"
        );
        assert_eq!(rig.set.value(rig.id("gain")), 1.0, "{number}: unchanged");
    }
}

#[test]
fn a_version_past_u64s_range_or_written_as_a_float_still_loads() {
    let rig = rig();
    for version in ["18446744073709551616", "1.0", "1e3", "2"] {
        let block = format!(r#"{{"seamStateVersion": {version}, "parameters": {{"freq": 500}}}}"#);
        assert!(rig.state.load(block.as_bytes()).is_ok(), "{version}");
    }
    for version in ["0.5", "1.5", "-1", "1e400"] {
        let block = format!(r#"{{"seamStateVersion": {version}, "parameters": {{}}}}"#);
        assert!(rig.state.load(block.as_bytes()).is_err(), "{version}");
    }
}

/// A participant that holds whatever section it was last handed.
struct Holds(Section);

impl Participant for Holds {
    fn capture(&self) -> Section {
        self.0.clone()
    }

    fn apply(&mut self, section: Option<&Section>) -> Result<(), ApplyError> {
        self.0 = section.cloned().unwrap_or(Section::Null);
        Ok(())
    }
}

/// A saved state whose session has one participant, `holds`, holding
/// `section`.
struct Holding {
    rig: Rig,
    ui: seam_session::SessionUi,
    holds: Rc<RefCell<Holds>>,
    registration: seam_session::Registration,
}

fn holding(section: Section) -> Holding {
    holding_of(set(), section)
}

fn holding_of(set: Arc<ParamSet>, section: Section) -> Holding {
    let rig = rig_of(set);
    let ui = rig.session.take_ui().expect("the first UI handle");
    let holds = Rc::new(RefCell::new(Holds(section)));
    let registration = ui.register("holds", &holds).expect("free");
    Holding {
        rig,
        ui,
        holds,
        registration,
    }
}

/// xorshift64*, for values with every bit pattern a finite double has.
fn finite_doubles(seed: u64, count: usize) -> Vec<f64> {
    let mut state = seed | 1;
    let mut values = Vec::with_capacity(count);
    while values.len() < count {
        state ^= state >> 12;
        state ^= state << 25;
        state ^= state >> 27;
        let value = f64::from_bits(state.wrapping_mul(0x2545_F491_4F6C_DD1D));
        if value.is_finite() {
            values.push(value);
        }
    }
    values
}

#[test]
fn save_load_save_is_a_fixpoint_for_any_double_in_a_section_or_a_kept_key() {
    // Every double, of every magnitude, in a participant's section and in
    // a key this build does not declare, saves the same bytes after a load.
    for seed in 1..=8 {
        let doubles = finite_doubles(seed, 256);
        let first = holding(Section::Null);
        let kept: serde_json::Map<String, serde_json::Value> = doubles
            .iter()
            .take(32)
            .enumerate()
            .map(|(index, &value)| (format!("newer{index}"), json!(value)))
            .collect();
        let block = json!({"seamStateVersion": 1, "parameters": kept}).to_string();
        assert!(first.rig.state.load(block.as_bytes()).is_ok());
        first.ui.tick();
        first.holds.borrow_mut().0 = json!(doubles);
        first.registration.changed();
        let saved = first.rig.state.save();

        let second = holding(Section::Null);
        assert!(second.rig.state.load(&saved).is_ok(), "seed {seed}");
        second.ui.tick();
        assert_eq!(
            second.holds.borrow().0,
            json!(doubles),
            "seed {seed}: every double read back exactly"
        );
        assert_eq!(second.rig.state.save(), saved, "seed {seed}: byte for byte");
    }
}

/// A section of `levels` nested arrays around one number.
fn nested(levels: usize) -> Section {
    let mut node = json!(1);
    for _ in 0..levels {
        node = Section::Array(vec![node]);
    }
    node
}

#[test]
fn the_deepest_section_a_capture_takes_saves_a_block_that_loads_and_a_deeper_one_is_refused() {
    let deepest = seam_session::MAX_SECTION_DEPTH;
    let first = holding(nested(deepest));
    assert!(!first.registration.refused());
    let saved = first.rig.state.save();
    let second = holding(Section::Null);
    assert!(
        second.rig.state.load(&saved).is_ok(),
        "{deepest} levels load"
    );
    second.ui.tick();
    assert_eq!(second.holds.borrow().0, nested(deepest));

    first.holds.borrow_mut().0 = nested(deepest + 1);
    first.registration.changed();
    assert!(first.registration.refused(), "one level deeper is refused");
    assert_eq!(
        first.rig.state.save(),
        saved,
        "the section before it stays saved, in a block that loads"
    );
}

/// Captures `section` into `holding`'s participant, as its owner would.
fn capture(holding: &Holding, section: Section) {
    holding.holds.borrow_mut().0 = section;
    holding.registration.changed();
}

#[test]
fn a_capture_that_would_pass_the_load_limit_is_refused_and_the_block_saved_before_stays() {
    // A participant that captured a 64 MiB string once made the plugin save a
    // block of 67,108,934 bytes, which the same build refused to load.
    let first = holding(json!("before"));
    let before = first.rig.state.save();
    capture(&first, json!("x".repeat(MAX_STATE_BYTES)));
    assert!(first.registration.refused());
    let saved = first.rig.state.save();
    assert_eq!(saved, before, "the section before it stays saved");
    assert!(holding(Section::Null).rig.state.load(&saved).is_ok());
}

/// The most bytes a declared parameter's value takes in the block: a minus,
/// `0.0000` and 17 digits.
const LONGEST_VALUE_BYTES: usize = 24;

/// A value of `range(-1.0, 1.0, ...)` that takes [`LONGEST_VALUE_BYTES`].
fn longest_value() -> f32 {
    let start = (-1.5e-5_f32).to_bits();
    (0..1_000)
        .map(|step| f32::from_bits(start + step))
        .find(|value| json!(f64::from(*value)).to_string().len() == LONGEST_VALUE_BYTES)
        .expect("a value of 17 digits after four zeros")
}

/// A set of `count` continuous parameters from -1 to 1, keyed `p0` on.
fn wide_set(count: usize) -> Arc<ParamSet> {
    ParamSet::from_decls((0..count).map(|index| {
        let key: &'static str = Box::leak(format!("p{index}").into_boxed_str());
        decl(key, "", range(-1.0, 1.0, 0.0, None), 0.0)
    }))
    .expect("valid")
}

#[test]
fn the_largest_capture_saves_a_block_at_the_limit_that_loads_and_one_byte_more_is_refused() {
    // Every term of the room at its longest, so a term counted short lets a
    // block past the limit and one counted long leaves room unused: each
    // parameter at its longest value, the largest version, and kept keys
    // whose escapes lengthen them.
    let set = wide_set(8);
    let first = holding_of(Arc::clone(&set), Section::Null);
    let state = &first.rig.state;
    let longest = longest_value();
    let mut parameters: serde_json::Map<String, serde_json::Value> = set
        .ids()
        .map(|id| (set.decl(id).key.to_owned(), json!(f64::from(longest))))
        .collect();
    parameters.insert("new\"er\\".into(), json!("\u{e9}\u{1}"));
    parameters.insert("\u{1}x".into(), json!([1, "\""]));
    let block = json!({"seamStateVersion": u64::MAX, "parameters": parameters,
                       "fut\"ure": ["\\", "\u{2}"], "\u{1f}other": {"k\"": 1}});
    assert!(state.load(block.to_string().as_bytes()).is_ok());
    first.ui.tick();
    assert!(set.ids().all(|id| set.value(id) == longest));
    assert_eq!(version_of(&state.save()), json!(u64::MAX));

    // Each character of the section adds one byte to the block, so `limit`
    // characters would make it exactly the limit.
    capture(&first, json!(""));
    let limit = MAX_STATE_BYTES - state.save().len();
    let taken = |length: usize| {
        capture(&first, json!("x".repeat(length)));
        !first.registration.refused()
    };
    // The room counts the parameters' closing brace twice, once in the frame
    // and once after the last value, so the largest capture is one short.
    let largest = (limit - 2..=limit + 1)
        .rev()
        .find(|&length| taken(length))
        .expect("a capture within two bytes of the limit");
    let saved = state.save();
    assert!(
        saved.len() <= MAX_STATE_BYTES,
        "{} bytes past the limit",
        saved.len() - MAX_STATE_BYTES
    );

    assert!(!taken(largest + 1), "one byte more is refused");
    assert_eq!(state.save(), saved, "and the block before it stays");

    let second = rig_of(set);
    assert!(
        second.state.load(&saved).is_ok(),
        "the block at the limit loads"
    );
    assert_eq!(second.state.save(), saved, "and saves back to itself");
}

/// A block of exactly `length` bytes: `head`, then `x` repeated, then `tail`.
fn padded(head: &str, length: usize, tail: &str) -> Vec<u8> {
    let mut block = head.as_bytes().to_vec();
    block.resize(length - tail.len(), b'x');
    block.extend_from_slice(tail.as_bytes());
    block
}

/// What a refused load must leave as it was: every value, the clock, the
/// session's sections and the block a save writes.
fn everything(rig: &Rig) -> (Vec<f32>, u64, Option<Section>, Vec<u8>) {
    let values = rig.set.ids().map(|id| rig.set.value(id)).collect();
    let stamp = rig.clock.stamp().stored();
    (values, stamp, rig.session.save(), rig.state.save())
}

#[test]
fn a_block_within_the_limit_that_would_save_back_past_it_is_refused_and_changes_nothing() {
    // A block of the limit, its parameters left out: written in full, the
    // parameters of a set of 40 once took it to 67,109,343 bytes.
    let rig = rig_of(wide_set(40));
    let kept = br#"{"seamStateVersion":1,"parameters":{"p3":0.5},"extra":{"a":1},"older":2}"#;
    assert!(rig.state.load(kept).is_ok());
    let before = everything(&rig);
    let block = padded(
        r#"{"seamStateVersion":1,"parameters":{},"extra":{"big":""#,
        MAX_STATE_BYTES,
        r#""}}"#,
    );
    assert_eq!(block.len(), MAX_STATE_BYTES);
    assert_eq!(rig.state.load(&block), Err(LoadError::TooLarge));
    assert_eq!(everything(&rig), before);

    // A block of 20 MiB whose kept key's numbers lengthen as they are written
    // back, `1e15` as `1000000000000000.0`, once saved 79,691,833 bytes.
    let numbers = vec!["1e15"; 20 * 1024 * 1024 / 5].join(",");
    let block = format!(r#"{{"seamStateVersion":1,"parameters":{{}},"future":[{numbers}]}}"#);
    assert!(block.len() < MAX_STATE_BYTES);
    assert_eq!(rig.state.load(block.as_bytes()), Err(LoadError::TooLarge));
    assert_eq!(everything(&rig), before);
}

/// A section of `length` characters of text.
fn text(length: usize) -> Section {
    json!("x".repeat(length))
}

#[test]
fn a_capture_taken_before_the_saved_state_is_made_is_judged_when_it_is_made() {
    // A plugin may register a participant before it makes its saved state, as
    // the Plugin example attaches learn. Its capture then has the whole block
    // for budget, and the saved state's room is less by the parameters.
    let set = set();
    let session = Session::new(&set);
    let ui = session.take_ui().expect("the first UI handle");
    let whole = Rc::new(RefCell::new(Holds(text(MAX_STATE_BYTES))));
    let whole_registration = ui.register("whole", &whole).expect("free");
    assert!(whole_registration.refused(), "past the whole block already");
    let near = Rc::new(RefCell::new(Holds(text(MAX_STATE_BYTES - 100))));
    let registration = ui.register("near", &near).expect("free");
    assert!(!registration.refused(), "within the whole block");

    let state = SavedState::new(&set, &session);
    assert!(registration.refused(), "past the room the parameters leave");
    assert!(whole_registration.refused());
    let saved = state.save();
    assert!(saved.len() <= MAX_STATE_BYTES);
    assert!(rig().state.load(&saved).is_ok());

    near.borrow_mut().0 = json!("small");
    registration.changed();
    assert!(!registration.refused(), "a capture that fits clears it");
}

#[test]
fn a_capture_taken_after_the_saved_state_is_made_is_refused_as_it_is_taken() {
    let first = holding(text(MAX_STATE_BYTES - 100));
    assert!(first.registration.refused());
    let saved = first.rig.state.save();
    assert!(saved.len() <= MAX_STATE_BYTES);
    assert!(rig().state.load(&saved).is_ok());
}

#[test]
fn a_restored_section_the_saved_state_drops_stays_dropped_through_its_apply() {
    // A section restored before the saved state is made, still waiting for
    // its apply, once came back at the apply's commit: 67,109,298 bytes.
    let set = set();
    let session = Session::new(&set);
    let ui = session.take_ui().expect("the first UI handle");
    let holds = Rc::new(RefCell::new(Holds(json!("before"))));
    let registration = ui.register("holds", &holds).expect("free");
    session.restore(Some(&json!({ "holds": text(MAX_STATE_BYTES - 100) })));

    let state = SavedState::new(&set, &session);
    assert!(registration.refused());
    let saved = state.save();
    assert!(saved.len() <= MAX_STATE_BYTES);
    ui.tick();
    assert_eq!(state.save(), saved, "the apply saves nothing back");
    assert_eq!(holds.borrow().0, json!("before"), "nor hands it over");
    assert!(rig().state.load(&saved).is_ok());

    registration.changed();
    assert!(!registration.refused());
    assert_eq!(session.save(), Some(json!({ "holds": "before" })));
}

/// A participant whose apply makes its state `bytes` characters of text,
/// whatever its section.
struct Grows {
    state: Section,
    bytes: usize,
}

impl Participant for Grows {
    fn capture(&self) -> Section {
        self.state.clone()
    }

    fn apply(&mut self, _: Option<&Section>) -> Result<(), ApplyError> {
        self.state = text(self.bytes);
        Ok(())
    }
}

#[test]
fn a_load_sets_its_budget_with_its_sections_so_an_apply_that_grows_is_refused_and_the_rest_stays() {
    // The result of a whole load and then a tick: each apply's capture is
    // judged against the budget the load set. Here the kept keys leave about
    // 44 MiB: the restored 16 MiB fits, and growing the other section to 32 MiB
    // does not.
    let rig = rig();
    let ui = rig.session.take_ui().expect("the first UI handle");
    let grows = Rc::new(RefCell::new(Grows {
        state: json!(0),
        bytes: 32 << 20,
    }));
    let grows_registration = ui.register("grows", &grows).expect("free");
    let holds = Rc::new(RefCell::new(Holds(Section::Null)));
    let holds_registration = ui.register("holds", &holds).expect("free");
    let block = json!({"seamStateVersion": 1, "parameters": {}, "future": text(20 << 20),
                       "extra": {"grows": 0, "holds": text(16 << 20)}});
    assert!(rig.state.load(block.to_string().as_bytes()).is_ok());
    ui.tick();

    assert!(grows_registration.refused(), "no room to grow into");
    assert!(!holds_registration.refused());
    let saved = rig.saved();
    assert_eq!(saved["extra"]["grows"], 0, "its raw section stays");
    assert_eq!(saved["extra"]["holds"], text(16 << 20));
    let saved = rig.state.save();
    assert!(saved.len() <= MAX_STATE_BYTES);
    assert!(self::rig().state.load(&saved).is_ok());
}

#[test]
fn the_unknown_keys_a_load_kept_take_their_share_of_the_limit() {
    let first = holding(json!(""));
    let quarter = MAX_STATE_BYTES / 4;
    let block = json!({"seamStateVersion": 1, "parameters": {"newer": "p".repeat(2 * quarter)},
                       "future": "f".repeat(quarter)});
    assert!(first.rig.state.load(block.to_string().as_bytes()).is_ok());
    first.ui.tick();
    capture(&first, json!("x".repeat(quarter)));
    assert!(
        first.registration.refused(),
        "a quarter that would fit alone, after three kept"
    );
    let saved = first.rig.state.save();
    assert!(saved.len() <= MAX_STATE_BYTES);
    assert!(holding(Section::Null).rig.state.load(&saved).is_ok());

    // A load without them gives their room back.
    let block = json!({"seamStateVersion": 1, "parameters": {}});
    assert!(first.rig.state.load(block.to_string().as_bytes()).is_ok());
    first.ui.tick();
    capture(&first, json!("x".repeat(quarter)));
    assert!(!first.registration.refused());
}

#[test]
fn an_older_block_is_migrated_before_it_loads_and_one_that_cannot_be_changes_nothing() {
    let set = set();
    let session = Session::new(&set);
    let state = SavedState::new(&set, &session).with_migration(3, |from, block| {
        let Some(serde_json::Value::Object(parameters)) = block.get_mut("parameters") else {
            return false;
        };
        // Version 2 renamed `level` to `gain`; version 3 halved `steps`.
        if from < 2
            && let Some(level) = parameters.remove("level")
        {
            parameters.insert("gain".to_owned(), level);
        }
        if from < 3 {
            match parameters.get("steps").and_then(serde_json::Value::as_f64) {
                Some(steps) => {
                    parameters.insert("steps".to_owned(), json!(steps / 2.0));
                }
                None => return false,
            }
        }
        true
    });
    let value = |key| set.value(set.id(key).expect("declared"));

    assert!(
        state
            .load(br#"{"seamStateVersion": 1, "parameters": {"level": 6, "steps": 9}}"#)
            .is_ok()
    );
    assert_eq!(
        (value("gain"), value("steps")),
        (6.0, 4.5),
        "both steps applied"
    );
    assert!(
        state
            .load(br#"{"seamStateVersion": 3, "parameters": {"level": 6, "steps": 9}}"#)
            .is_ok()
    );
    assert_eq!(
        (value("gain"), value("steps")),
        (0.0, 9.0),
        "a current block loads as it is"
    );
    let saved: serde_json::Value = serde_json::from_slice(&state.save()).expect("JSON");
    assert_eq!(
        saved["seamStateVersion"], 3,
        "and the current version is written"
    );

    let before = state.save();
    assert_eq!(
        state.load(br#"{"seamStateVersion": 2, "parameters": {"gain": 1}}"#),
        Err(LoadError::MigrationFailed),
        "one the migration cannot upgrade is not recognised"
    );
    assert_eq!(state.save(), before, "and changes nothing");
}

#[test]
fn hostile_blocks_through_a_migration_keep_every_promise() {
    // A migration sees only a block whose version passed and is older, as
    // the host sent it. Whatever it does to the block, one that is then not
    // recognised changes nothing, and one that loads saves back to itself.
    let set = set();
    let session = Session::new(&set);
    let seen = Arc::new(std::sync::Mutex::new(Vec::new()));
    let migrate = {
        let seen = Arc::clone(&seen);
        move |from: u64, block: &mut serde_json::Map<String, serde_json::Value>| {
            seen.lock().expect("unpoisoned").push(from);
            match block.get("do").and_then(serde_json::Value::as_str) {
                Some("drop") => drop(block.remove("parameters")),
                Some("break") => drop(block.insert("parameters".into(), json!([1]))),
                Some("refuse") => return false,
                _ => {}
            }
            true
        }
    };
    let state = SavedState::new(&set, &session).with_migration(2, migrate);
    let gain = set.id("gain").expect("declared");
    set.program().load().restore(gain, 7.0);
    let before = (set.value(gain), state.save());
    for (block, refused) in [
        (
            r#"{"seamStateVersion": 0, "parameters": {}}"#,
            Some(LoadError::BadVersion),
        ),
        (
            r#"{"seamStateVersion": 1e400, "parameters": {}}"#,
            Some(LoadError::BadVersion),
        ),
        (
            r#"{"seamStateVersion": 1, "parameters": 5}"#,
            Some(LoadError::BadParameters),
        ),
        (
            r#"{"seamStateVersion": 1, "do": "drop", "parameters": {}}"#,
            Some(LoadError::BadParameters),
        ),
        (
            r#"{"seamStateVersion": 1, "do": "break", "parameters": {}}"#,
            Some(LoadError::BadParameters),
        ),
        (
            r#"{"seamStateVersion": 1, "do": "refuse", "parameters": {}}"#,
            Some(LoadError::MigrationFailed),
        ),
        (
            r#"{"seamStateVersion": 1, "do": 1e400, "parameters": {"gain": [[]]}, "extra": 4}"#,
            None,
        ),
        (
            r#"{"seamStateVersion": 3, "do": "refuse", "parameters": {}}"#,
            None,
        ),
    ] {
        let result = state.load(block.as_bytes());
        assert_eq!(result.err(), refused, "{block}");
        if refused.is_some() {
            assert_eq!((set.value(gain), state.save()), before, "{block}: changed");
            continue;
        }
        let saved = state.save();
        let again = SavedState::new(&set, &Session::new(&set)).with_migration(2, |_, _| true);
        assert!(again.load(&saved).is_ok(), "{block}: its save loads");
        assert_eq!(again.save(), saved, "{block}: and saves back to itself");
        set.program().load().restore(gain, 7.0);
        assert!(state.load(&before.1).is_ok());
    }
    assert_eq!(
        *seen.lock().expect("unpoisoned"),
        [1, 1, 1, 1, 1],
        "only older blocks of a whole version reach it"
    );
}

/// Version 2 of a plugin renamed version 1's `level` to `gain`, migrating
/// as the docs say a migration must: it inserts and never overwrites.
fn renamed(from: u64, block: &mut serde_json::Map<String, serde_json::Value>) -> bool {
    let Some(serde_json::Value::Object(parameters)) = block.get_mut("parameters") else {
        return false;
    };
    if from < 2
        && let Some(level) = parameters.remove("level")
    {
        parameters.entry("gain").or_insert(level);
    }
    true
}

/// A saved state of one parameter, `key`, writing version 1, or version 2
/// with [`renamed`].
fn build(key: &'static str, newer: bool) -> SavedState {
    let set =
        ParamSet::from_decls([decl(key, "", range(-24.0, 24.0, 0.0, None), 0.0)]).expect("valid");
    let state = SavedState::new(&set, &Session::new(&set));
    if newer {
        state.with_migration(2, renamed)
    } else {
        state
    }
}

fn version_of(block: &[u8]) -> serde_json::Value {
    serde_json::from_slice::<serde_json::Value>(block).expect("JSON")["seamStateVersion"].clone()
}

#[test]
fn a_newer_builds_block_keeps_its_version_and_values_through_an_older_builds_save() {
    let newer = build("gain", true);
    let gain = newer.set().id("gain").expect("declared");
    newer.set().program().load().restore(gain, 6.0);
    let block = newer.save();

    let older = build("level", false);
    assert!(older.load(&block).is_ok());
    let block = older.save();
    assert_eq!(
        version_of(&block),
        2,
        "the newer version, not the older build's own"
    );

    let again = build("gain", true);
    assert!(again.load(&block).is_ok());
    assert_eq!(
        again.set().value(gain),
        6.0,
        "not migrated as a version 1 block"
    );
}

#[test]
fn a_fresh_state_and_one_that_loaded_an_older_block_save_their_own_version() {
    let newer = build("gain", true);
    assert_eq!(version_of(&newer.save()), 2, "fresh");
    assert!(
        newer
            .load(br#"{"seamStateVersion": 3, "parameters": {"gain": 1}}"#)
            .is_ok()
    );
    assert_eq!(version_of(&newer.save()), 3, "a newer block's");
    assert!(
        newer
            .load(br#"{"seamStateVersion": 1, "parameters": {"level": 6}}"#)
            .is_ok()
    );
    assert_eq!(version_of(&newer.save()), 2, "the last load's, upgraded");
    assert_eq!(
        newer.set().value(newer.set().id("gain").expect("declared")),
        6.0
    );

    let older = build("level", false);
    assert!(
        older
            .load(br#"{"seamStateVersion": 1, "parameters": {"level": 6}}"#)
            .is_ok()
    );
    assert_eq!(version_of(&older.save()), 1);
}

#[test]
fn json_whitespace_around_the_object_is_part_of_the_block() {
    let rig = rig();
    let block = String::from_utf8(rig.state.save()).expect("UTF-8");
    for padded in [
        format!(" \n\t\r{block}"),
        format!("{block}\r\n"),
        format!("{block} \t\n\0\0"),
    ] {
        assert!(rig.state.load(padded.as_bytes()).is_ok(), "{padded:?}");
    }
    assert_eq!(
        rig.state.load(format!("{block}\0 ").as_bytes()),
        Err(LoadError::NotJson),
        "but not after NUL padding"
    );
}
