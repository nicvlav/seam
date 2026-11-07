//! Hostile saved blocks: random, structured and mutated, against a plugin
//! with parameters and session participants. Whatever a host hands over,
//! a load never panics, either applies or leaves everything as it was, gives
//! the same state whatever was there before, and saves a block that loads
//! back to itself.
//!
//! The cases come from a seeded generator, so a failure names its case and
//! replays; `PROPTEST_CASES`, which CI raises on pushes to main, scales them
//! as it scales the property tests.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use seam_host::{LoadError, SavedState};
use seam_params::{Kind, ParamDecl, ParamSet};
use seam_session::{ApplyError, Participant, Registration, Section, Session, SessionUi};
use serde_json::json;

const MODES: &[&str] = &["low", "band", "high"];

fn decl(key: &'static str, kind: Kind, default: f32) -> ParamDecl {
    ParamDecl::new(key, key, kind, default)
}

fn set() -> Arc<ParamSet> {
    ParamSet::from_decls([
        decl(
            "freq",
            Kind::continuous(20.0, 20_000.0, 0.0, Some(1_000.0)),
            1_000.0,
        ),
        decl("gain", Kind::continuous(-24.0, 24.0, 0.0, None), 0.0),
        decl("on", Kind::Bool, 1.0),
        decl("mode", Kind::Choice(MODES), 1.0),
        decl("steps", Kind::continuous(0.0, 10.0, 3.0, None), 3.0),
    ])
    .expect("valid")
}

/// A participant that takes a string and refuses anything else.
struct Notes(String);

impl Participant for Notes {
    fn capture(&self) -> Section {
        if self.0.is_empty() {
            Section::Null
        } else {
            json!(self.0)
        }
    }

    fn apply(&mut self, section: Option<&Section>) -> Result<(), ApplyError> {
        self.0 = match section {
            None => String::new(),
            Some(Section::String(text)) => text.clone(),
            Some(_) => return Err(ApplyError::new("not a string")),
        };
        Ok(())
    }
}

/// A participant that takes whatever it is handed.
struct Anything(Section);

impl Participant for Anything {
    fn capture(&self) -> Section {
        self.0.clone()
    }

    fn apply(&mut self, section: Option<&Section>) -> Result<(), ApplyError> {
        self.0 = section.cloned().unwrap_or(Section::Null);
        Ok(())
    }
}

/// A plugin's saved state, with its session's participants.
struct Plugin {
    set: Arc<ParamSet>,
    state: SavedState,
    ui: SessionUi,
    _notes: (Rc<RefCell<Notes>>, Registration),
    _anything: (Rc<RefCell<Anything>>, Registration),
}

/// A plugin whose state before the load differs with `prior`.
fn plugin(prior: u32) -> Plugin {
    let set = set();
    let session = Session::new(&set);
    let ui = session.take_ui().expect("the first UI handle");
    let notes = Rc::new(RefCell::new(Notes(String::new())));
    let notes_registration = ui.register("notes", &notes).expect("free");
    let anything = Rc::new(RefCell::new(Anything(Section::Null)));
    let anything_registration = ui.register("anything", &anything).expect("free");
    let state = SavedState::new(&set, &session);
    if prior > 0 {
        let load = set.program().load();
        for (index, id) in set.ids().enumerate() {
            load.restore(id, prior as f32 * (index as f32 + 1.0));
        }
        notes.borrow_mut().0 = format!("prior {prior}");
        notes_registration.changed();
        anything.borrow_mut().0 = json!([prior]);
        anything_registration.changed();
        ui.tick();
    }
    Plugin {
        set,
        state,
        ui,
        _notes: (notes, notes_registration),
        _anything: (anything, anything_registration),
    }
}

impl Plugin {
    fn values(&self) -> Vec<u32> {
        self.set
            .ids()
            .map(|id| self.set.value(id).to_bits())
            .collect()
    }
}

/// Every promise a load makes, for one block; whether it was recognised.
fn check(block: &[u8], case: &str) -> bool {
    let (a, b) = (plugin(0), plugin(3));
    let before = (b.values(), b.state.save(), b.set.clock().reader().stamp());
    let (took_a, took_b) = (a.state.load(block), b.state.load(block));
    let refusal = |took: &Result<(), LoadError>| took.as_ref().err().map(std::mem::discriminant);
    assert_eq!(
        refusal(&took_a),
        refusal(&took_b),
        "{case}: recognised or not whatever was there"
    );
    if took_b.is_err() {
        assert_eq!(
            (b.values(), b.state.save(), b.set.clock().reader().stamp()),
            before,
            "{case}: a block not recognised changed something"
        );
        return false;
    }
    assert_eq!(a.values(), b.values(), "{case}: the same values");
    assert_eq!(a.state.save(), b.state.save(), "{case}: the same block");
    a.ui.tick();
    b.ui.tick();
    let saved = a.state.save();
    assert_eq!(saved, b.state.save(), "{case}: the same block once applied");
    let c = plugin(7);
    assert!(c.state.load(&saved).is_ok(), "{case}: a saved block loads");
    c.ui.tick();
    assert_eq!(c.state.save(), saved, "{case}: and saves back to itself");
    true
}

/// xorshift64*, seeded per case.
struct Random(u64);

impl Random {
    fn new(seed: u64) -> Self {
        Random(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1)
    }

    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    fn below(&mut self, count: usize) -> usize {
        (self.next() % count as u64) as usize
    }

    fn pick<'a>(&mut self, items: &[&'a str]) -> &'a str {
        items[self.below(items.len())]
    }
}

const KEYS: &[&str] = &[
    "gain", "mode", "on", "steps", "freq", "notes", "anything", "a", "", "\\u0000",
];

/// Numbers a host or a newer build may write, the hostile ones included.
const NUMBERS: &[&str] = &[
    "0",
    "-0",
    "1",
    "2",
    "-1",
    "0.5",
    "1.6",
    "1e39",
    "-1e39",
    "1e400",
    "-1e400",
    "1e-400",
    "18446744073709551616",
    "-9223372036854775809",
    "1E+2",
];

/// A JSON value as text, `depth` levels at most.
fn value(random: &mut Random, depth: u32) -> String {
    match random.below(if depth == 0 { 6 } else { 8 }) {
        0 => random.pick(&["null", "true", "false"]).to_owned(),
        1 => random.pick(NUMBERS).to_owned(),
        2 => format!("{}", random.next() as i64),
        3 => format!("{:?}", f64::from_bits(random.next()))
            .replace("NaN", "null")
            .replace("inf", "1e400"),
        4 => format!("\"{}\"", random.pick(KEYS)),
        5 => format!(
            "\"{}\"",
            random.pick(&["x", "\\\"", "\\\\", "\\n", "é", "1e400"])
        ),
        6 => {
            let items: Vec<String> = (0..random.below(5))
                .map(|_| value(random, depth - 1))
                .collect();
            format!("[{}]", items.join(","))
        }
        _ => object(random, depth - 1),
    }
}

fn object(random: &mut Random, depth: u32) -> String {
    let fields: Vec<String> = (0..random.below(6))
        .map(|_| format!("\"{}\":{}", random.pick(KEYS), value(random, depth)))
        .collect();
    format!("{{{}}}", fields.join(","))
}

/// A block shaped as a saved one, every part of it hostile in turn.
fn block(random: &mut Random) -> String {
    let version = match random.below(4) {
        0 => "1".to_owned(),
        1 => random
            .pick(&[
                "2",
                "0",
                "-1",
                "1.0",
                "\"1\"",
                "null",
                "1e400",
                "18446744073709551616",
            ])
            .to_owned(),
        2 => format!("{}", random.next()),
        _ => value(random, 1),
    };
    let parameters = if random.below(8) == 0 {
        value(random, 2)
    } else {
        object(random, 2)
    };
    let extra = match random.below(3) {
        0 => String::new(),
        1 => format!(",\"extra\":{}", object(random, 3)),
        _ => format!(",\"extra\":{}", value(random, 3)),
    };
    // Drawn last, so the parts above are as they were before it existed.
    let plugin = match random.below(4) {
        0 | 1 => String::new(),
        2 => format!(
            ",\"pluginStateVersion\":{}",
            random.pick(&["0", "1", "2", "1.0"])
        ),
        _ => format!(",\"pluginStateVersion\":{}", value(random, 1)),
    };
    format!("{{\"seamStateVersion\":{version}{plugin},\"parameters\":{parameters}{extra}}}")
}

/// How many cases each property runs.
fn cases() -> u64 {
    std::env::var("PROPTEST_CASES")
        .ok()
        .and_then(|cases| cases.parse().ok())
        .unwrap_or(256)
        * 2
}

#[test]
fn structured_blocks_keep_every_promise() {
    let recognised = (0..cases())
        .filter(|&case| {
            let text = block(&mut Random::new(case));
            check(text.as_bytes(), &format!("structured case {case}: {text}"))
        })
        .count() as u64;
    assert!(
        recognised * 4 > cases(),
        "{recognised} recognised: too few to test"
    );
}

#[test]
fn random_bytes_never_panic_and_keep_every_promise() {
    for case in 0..cases() {
        let mut random = Random::new(case ^ 0xB10C);
        let bytes: Vec<u8> = (0..random.below(200))
            .map(|_| random.next() as u8)
            .collect();
        check(&bytes, &format!("random case {case}: {bytes:?}"));
    }
}

#[test]
fn cut_flipped_and_padded_blocks_keep_every_promise() {
    for case in 0..cases() {
        let mut random = Random::new(case ^ 0xF11B);
        let text = block(&mut random).into_bytes();
        let cut = &text[..random.below(text.len() + 1)];
        check(cut, &format!("cut case {case}: {cut:?}"));
        let mut flipped = text.clone();
        let at = random.below(flipped.len());
        flipped[at] ^= random.next() as u8 | 1;
        check(&flipped, &format!("flipped case {case}: {flipped:?}"));
        let mut padded = text.clone();
        padded.extend(std::iter::repeat_n(0, random.below(64)));
        if random.below(2) == 0 {
            padded.extend_from_slice(b" {}");
        }
        check(&padded, &format!("padded case {case}: {padded:?}"));
    }
}
