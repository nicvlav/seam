//! The run-time section's room: a save writes the entries that fit, in path
//! order, and flags the rest; a load counts its entries at least as long as
//! a save writes them back. The room is passed in, since a block at the
//! real limit is too slow to build in a debug test.

use seam_learn::{Control, Curve, Mapping, Source, encode_mapping};
use seam_params::{AnyParam, Declarer, Float, FloatDecl, ParamSet};
use serde_json::{Value as Json, json};

use super::{FRAME_BYTES, Loaded, RunTimeSaved, entry_bytes};

/// A volume `set`'s instance makes at run time, named under `scope`.
fn named(saved: &RunTimeSaved, set: &ParamSet, scope: &str) -> Float {
    let decl = FloatDecl::new("volume", "Volume", -60.0..=12.0, 0.0);
    let (volume, made) = Declarer::run_time_one(set, Box::new(|_| None), decl);
    let naming = saved.name(scope, &made, 0, false);
    assert!(naming.replaced.is_empty());
    volume
}

/// The paths a written section holds, in order.
fn paths(block: &str) -> Vec<String> {
    let section: Json = serde_json::from_str(&format!("{{{}}}", &block[1..])).expect("JSON");
    section["runTime"]
        .as_object()
        .expect("an object")
        .keys()
        .cloned()
        .collect()
}

#[test]
fn a_save_writes_the_entries_that_fit_in_path_order_and_flags_the_rest() {
    let set = ParamSet::from_decls([]).expect("empty");
    let saved = RunTimeSaved::default();
    let _params: Vec<Float> = ["g/3", "g/1", "g/2"]
        .into_iter()
        .map(|scope| named(&saved, &set, scope))
        .collect();

    let mut whole = String::new();
    saved.write(&mut whole, usize::MAX);
    assert_eq!(paths(&whole), ["g/1/volume", "g/2/volume", "g/3/volume"]);
    assert_eq!(saved.take_faults(), (false, false));

    // Room for the frame and two entries at their longest.
    let room = FRAME_BYTES + 2 * entry_bytes("g/1/volume", None);
    let mut short = String::new();
    saved.write(&mut short, room);
    assert_eq!(paths(&short), ["g/1/volume", "g/2/volume"]);
    assert!(short.len() <= room);
    assert_eq!(saved.take_faults(), (false, true), "a save left short");

    // No room for even one: the key is left out.
    let mut none = String::new();
    saved.write(&mut none, FRAME_BYTES);
    assert!(none.is_empty(), "{none}");
    assert_eq!(saved.take_faults(), (false, true));
}

#[test]
fn an_entry_with_no_room_is_passed_over_and_a_later_one_that_fits_is_written() {
    let set = ParamSet::from_decls([]).expect("empty");
    let saved = RunTimeSaved::default();
    let first = named(&saved, &set, "a");
    let _second = named(&saved, &set, "b");
    let mapping = Mapping::new(Source::any(Control::Cc(7)))
        .with_curve(Curve::new(2.0, 0.25, 0.75).expect("a valid curve"));
    saved.set_learned(&first.erased(), &mapping);

    let room = FRAME_BYTES + entry_bytes("b/volume", None);
    let mut block = String::new();
    saved.write(&mut block, room);
    assert_eq!(paths(&block), ["b/volume"], "a/volume's mapping takes more");
    assert_eq!(saved.take_faults(), (false, true));
}

#[test]
fn a_load_counts_its_entries_at_least_as_long_as_a_save_writes_them_back() {
    let mapping = Mapping::new(Source::any(Control::Nrpn(300)))
        .with_curve(Curve::new(0.5, 0.0, 1.0).expect("a valid curve").inverted());
    let section = json!({
        "g/1/volume": { "value": -1.234_567_890_123_456_7e-300 },
        "g/2/on": { "value": true },
        "g/3/mode": { "value": 2, "learned": encode_mapping("g/3/mode", &mapping) },
        "g/4/learned": { "learned": encode_mapping("g/4/learned", &mapping) },
        "text": { "value": "not a number" },
        "empty": {},
        "not an object": 1,
    });
    let loaded = Loaded::read(Some(section));
    let counted = loaded.bytes();

    let saved = RunTimeSaved::default();
    saved.hold(loaded, 0);
    let mut block = String::new();
    saved.write(&mut block, counted);
    assert_eq!(
        paths(&block),
        ["g/1/volume", "g/2/on", "g/3/mode", "g/4/learned"],
        "each read back as loaded; the rest dropped"
    );
    assert!(block.len() <= counted, "{} > {counted}", block.len());
    assert_eq!(
        saved.take_faults(),
        (false, false),
        "it fits the room it counted"
    );
}

#[test]
fn nothing_named_or_held_writes_no_section_and_counts_nothing() {
    let saved = RunTimeSaved::default();
    let mut block = String::new();
    saved.write(&mut block, usize::MAX);
    assert!(block.is_empty());
    assert_eq!(Loaded::read(None).bytes(), 0);
    assert_eq!(Loaded::read(Some(json!({}))).bytes(), 0);
}
