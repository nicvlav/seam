//! Saving, loading and saving again writes the same digits for every f64 in a
//! section and in a kept key.

use std::cell::RefCell;
use std::rc::Rc;

use seam_params::ParamSetBuilder;
use seam_session::{ApplyError, Participant, Section, Session};
use serde_json::json;

/// A participant that keeps whatever section it is given.
struct Fixed(Section);

impl Participant for Fixed {
    fn capture(&self) -> Section {
        self.0.clone()
    }

    fn apply(&mut self, section: Option<&Section>) -> Result<(), ApplyError> {
        self.0 = section.cloned().unwrap_or(Section::Null);
        Ok(())
    }
}

/// A small deterministic generator.
struct Bits(u64);

impl Bits {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    /// A uniform f64 in [0, 1).
    fn unit(&mut self) -> f64 {
        (self.next() >> 11) as f64 / (1_u64 << 53) as f64
    }

    /// Any finite f64.
    fn finite(&mut self) -> f64 {
        loop {
            let value = f64::from_bits(self.next());
            if value.is_finite() {
                return value;
            }
        }
    }
}

/// Parses `text` as a host's load does, restores it, and saves again.
fn through(session: &Session, text: &str) -> String {
    let extra: Section = serde_json::from_str(text).expect("valid JSON");
    session.restore(Some(&extra));
    session.save().expect("sections").to_string()
}

#[test]
fn saving_loading_and_saving_again_writes_the_same_digits() {
    let mut bits = Bits(0x9E37_79B9_7F4A_7C15);
    let mut values = Vec::new();
    for _ in 0..4_000 {
        values.push(bits.unit());
    }
    for _ in 0..1_000 {
        values.push(bits.finite());
    }
    let set = ParamSetBuilder::new().build();
    let session = Session::new(&set);
    // No participant holds these keys, so they are kept ones, saved back
    // verbatim, as a section a newer build wrote would be.
    let first = Section::Object(
        values
            .chunks(50)
            .enumerate()
            .map(|(index, chunk)| (format!("k{index:03}"), json!(chunk)))
            .collect(),
    );
    let text = first.to_string();
    let second = through(&session, &text);
    assert_eq!(second, text, "the first reload changed a digit");
    assert_eq!(through(&session, &second), text, "the second reload did");
    let reloaded: Section = serde_json::from_str(&second).expect("valid JSON");
    for (key, section) in reloaded.as_object().expect("an object") {
        let index: usize = key[1..].parse().expect("a number");
        let original = &values[index * 50..(index * 50 + 50).min(values.len())];
        for (got, want) in section.as_array().expect("an array").iter().zip(original) {
            assert_eq!(got.as_f64().map(f64::to_bits), Some(want.to_bits()));
        }
    }
}

#[test]
fn a_participants_f64_section_survives_a_restore_unchanged() {
    let mut bits = Bits(7);
    let set = ParamSetBuilder::new().build();
    let session = Session::new(&set);
    let ui = session.take_ui().expect("the first UI handle");
    let participant = Rc::new(RefCell::new(Fixed(Section::Null)));
    let _registration = ui.register("numbers", &participant).expect("free");
    let values: Vec<f64> = (0..2_000).map(|_| bits.unit()).collect();
    let text = json!({ "numbers": values }).to_string();
    let saved = through(&session, &text);
    ui.tick();
    assert_eq!(saved, text);
    assert_eq!(session.save().expect("a section").to_string(), text);
}
