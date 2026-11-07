//! The parameter set: stores, change counters, declaration order and host ids.

use std::sync::Arc;

use seam_params::{
    DeclError, Kind, ParamDecl, ParamSet, ParamSetBuilder, Range, RangeError, Reading,
};
use seam_testkit::sync::Latch;

const LABELS: [&str; 3] = ["low", "mid", "high"];

fn param(key: &'static str, group: &'static str) -> ParamDecl {
    ParamDecl::new(key, key, Kind::continuous(0.0, 10.0, 1.0, None), 4.0).with_group(group)
}

/// A declaration whose kind and default the test chooses.
fn of(key: &'static str, group: &'static str, kind: Kind, default: f32) -> ParamDecl {
    ParamDecl::new(key, key, kind, default).with_group(group)
}

fn range(start: f32, end: f32, interval: f32, skew_centre: Option<f32>) -> Kind {
    Kind::continuous(start, end, interval, skew_centre)
}

fn build(decls: &[ParamDecl]) -> Arc<ParamSet> {
    let mut builder = ParamSetBuilder::new();
    for &decl in decls {
        builder.add(decl).expect("a valid declaration");
    }
    builder.build()
}

fn keys(set: &ParamSet) -> Vec<&'static str> {
    set.ids().map(|id| set.decl(id).key).collect()
}

#[test]
fn the_order_is_depth_first_with_a_groups_own_parameters_first() {
    let set = build(&[
        param("a", ""),
        param("b", "Mix"),
        param("c", ""),
        param("d", "Mix/Sides"),
        param("e", "Mix"),
        param("f", "Fx"),
        param("g", "Mix/Sides"),
    ]);
    assert_eq!(keys(&set), ["a", "c", "b", "e", "d", "g", "f"]);

    // A subgroup made after a sibling group still lists inside its parent.
    let set = build(&[param("b", "Mix"), param("f", "Fx"), param("d", "Mix/Sides")]);
    assert_eq!(keys(&set), ["b", "d", "f"]);
}

#[test]
fn a_parameter_is_found_by_key_and_by_its_host_index() {
    let set = build(&[param("b", "Mix"), param("a", "")]);
    let a = set.id("a").expect("declared");
    assert_eq!(a.index(), 0);
    assert_eq!(set.id_at(0), Some(a));
    assert_eq!(set.id_at(2), None);
    assert_eq!(set.id("missing"), None);
    assert_eq!(set.len(), 2);
}

#[test]
fn bad_declarations_are_refused_in_every_build() {
    let long = "a_key_of_thirty_two_bytes_long__";
    assert_eq!(long.len(), 32);
    let cases: [(ParamDecl, DeclError); 14] = [
        (param("", ""), DeclError::EmptyKey),
        (param("a/b", ""), DeclError::KeyHasSlash("a/b")),
        (param(long, ""), DeclError::KeyTooLong(long)),
        (param("taken", ""), DeclError::DuplicateKey("taken")),
        (param("Mix", ""), DeclError::KeyIsGroup("Mix")),
        (param("x", "taken"), DeclError::KeyIsGroup("taken")),
        (param("Self", "Self"), DeclError::KeyIsGroup("Self")),
        (
            param("y", "Mix//Sides"),
            DeclError::EmptyGroupName {
                key: "y",
                group: "Mix//Sides",
            },
        ),
        (
            of("one", "", Kind::Choice(&LABELS[..1]), 4.0),
            DeclError::TooFewChoices("one"),
        ),
        // A label nobody can type, or one that reads the same text as
        // another, so parsing would pick the first.
        (
            of("blank", "", Kind::Choice(&["low", ""]), 0.0),
            DeclError::EmptyChoiceLabel("blank"),
        ),
        (
            of("spaces", "", Kind::Choice(&["low", "  "]), 0.0),
            DeclError::EmptyChoiceLabel("spaces"),
        ),
        (
            of("leading", "", Kind::Choice(&["low", " Peak"]), 0.0),
            DeclError::PaddedChoiceLabel {
                key: "leading",
                label: " Peak",
            },
        ),
        (
            of("trailing", "", Kind::Choice(&["Peak", "Peak "]), 0.0),
            DeclError::PaddedChoiceLabel {
                key: "trailing",
                label: "Peak ",
            },
        ),
        (
            of("twice", "", Kind::Choice(&["low", "high", "Low"]), 0.0),
            DeclError::DuplicateChoiceLabel {
                key: "twice",
                label: "Low",
            },
        ),
    ];
    for (decl, expected) in cases {
        let mut builder = ParamSetBuilder::new();
        builder.add(param("taken", "Mix")).expect("valid");
        assert_eq!(
            builder.add(decl).err(),
            Some(expected.clone()),
            "{expected}"
        );
        // The refusal changed nothing: the builder goes on as before.
        builder.add(param("after", "")).expect("still accepts");
        assert_eq!(keys(&builder.build()), ["after", "taken"]);
    }
}

#[test]
fn a_range_is_refused_unless_it_can_be_snapped_to_and_converted() {
    let nan = f32::NAN;
    let cases = [
        (f32::NEG_INFINITY, 1.0, 0.0, None, RangeError::NotFinite),
        (0.0, nan, 0.0, None, RangeError::NotFinite),
        (0.0, f32::INFINITY, 0.0, None, RangeError::NotFinite),
        (5.0, 5.0, 0.0, None, RangeError::StartNotBelowEnd),
        (1.0, 0.0, 0.0, None, RangeError::StartNotBelowEnd),
        (0.0, 1.0, -0.1, None, RangeError::BadInterval),
        (0.0, 1.0, nan, None, RangeError::BadInterval),
        (0.0, 1.0, f32::INFINITY, None, RangeError::BadInterval),
        (0.0, 1.0, 0.0, Some(nan), RangeError::SkewCentreOutside),
        (0.0, 1.0, 0.0, Some(0.0), RangeError::SkewCentreOutside),
        (0.0, 1.0, 0.0, Some(1.0), RangeError::SkewCentreOutside),
        (0.0, 1.0, 0.0, Some(2.0), RangeError::SkewCentreOutside),
    ];
    for (start, end, interval, centre, expected) in cases {
        assert_eq!(
            Range::new(start, end, interval, centre),
            Err(expected),
            "{start} {end} {interval} {centre:?}"
        );
    }
    let range = Range::new(-1.0, 1.0, 0.5, Some(0.25)).expect("valid");
    assert_eq!(
        (
            range.start(),
            range.end(),
            range.interval(),
            range.skew_centre()
        ),
        (-1.0, 1.0, 0.5, Some(0.25))
    );
}

#[test]
fn a_refused_declaration_registers_neither_its_key_nor_its_groups() {
    let mut builder = ParamSetBuilder::new();
    // Refused for its labels, after its key and group names were checked.
    let bad = of("late", "Fx/Out", Kind::Choice(&LABELS[..1]), 4.0);
    assert!(builder.add(bad).is_err());
    // The key is free again, and so are the group names.
    builder.add(param("late", "")).expect("the key is free");
    builder
        .add(param("Fx", ""))
        .expect("no group took the name");
    builder
        .add(param("Out", ""))
        .expect("no group took the name");
    assert_eq!(keys(&builder.build()), ["late", "Fx", "Out"]);
}

#[test]
fn the_bypass_must_be_a_bool() {
    let decl = param("bypass", "").as_bypass();
    assert_eq!(
        ParamSetBuilder::new().add(decl).err(),
        Some(DeclError::BypassNotBool("bypass"))
    );
}

#[test]
fn at_most_one_parameter_is_the_bypass() {
    let bypass = |key| of(key, "", Kind::Bool, 4.0).as_bypass();
    let mut builder = ParamSetBuilder::new();
    builder.add(param("gain", "")).expect("valid");
    builder.add(bypass("bypass")).expect("the first bypass");
    assert_eq!(
        builder.add(bypass("other")).err(),
        Some(DeclError::SecondBypass("other"))
    );
    let set = builder.build();
    assert_eq!(set.bypass(), set.id("bypass"));
}

#[test]
fn every_store_starts_at_its_snapped_default() {
    let set = build(&[
        of("stepped", "", range(0.0, 10.0, 3.0, None), 7.4),
        of("nan", "", range(2.0, 4.0, 0.0, None), f32::NAN),
        of("choice", "", Kind::Choice(&LABELS), 1.6),
    ]);
    let value = |key| set.value(set.id(key).expect("declared"));
    assert_eq!(value("stepped"), 6.0);
    assert_eq!(value("nan"), 2.0);
    assert_eq!(value("choice"), 2.0);
    for id in set.ids() {
        assert_eq!(set.changes(id), 0);
    }
}

#[test]
fn a_host_write_sanitises_snaps_and_always_publishes() {
    let set = build(&[param("p", "")]);
    let audio = set.audio();
    let id = set.id("p").expect("declared");

    audio.host_write(id, 0.66);
    assert_eq!(set.value(id), 7.0);
    assert_eq!(set.changes(id), 1);
    assert!((set.normalized(id) - 0.7).abs() < 1e-6);

    // No equality check: the same value publishes again.
    audio.host_write(id, 0.66);
    assert_eq!(set.changes(id), 2);

    // Not finite becomes 0, the range start.
    audio.host_write(id, f64::NAN);
    assert_eq!(set.value(id), 0.0);
    assert_eq!(set.changes(id), 3);
}

#[test]
fn a_learned_write_keeps_the_value_for_nan() {
    let set = build(&[param("p", "")]);
    let audio = set.audio();
    let id = set.id("p").expect("declared");

    audio.learned_write(id, f64::INFINITY);
    assert_eq!(set.value(id), 10.0);
    assert_eq!(set.changes(id), 1);

    audio.learned_write(id, f64::NAN);
    assert_eq!(set.value(id), 10.0);
    assert_eq!(set.changes(id), 1, "NaN writes nothing");
}

#[test]
fn a_load_marks_the_restore_stored_before_writing_real_units() {
    let set = build(&[of(
        "freq",
        "",
        range(20.0, 20_000.0, 0.0, Some(1_000.0)),
        1_000.0,
    )]);
    let id = set.id("freq").expect("declared");
    let clock = std::sync::Arc::clone(set.clock());
    let reader = clock.reader();

    // The stored count moves before the first parameter write.
    let load = set.program().load();
    assert_eq!(reader.stamp().stored(), 1);
    assert_eq!(set.changes(id), 0);

    // Real units straight in, with no 0..1 round trip on a skewed range.
    load.restore(id, 1_234.567);
    assert_eq!(set.value(id), 1_234.567);
    assert_eq!(set.changes(id), 1);

    load.restore(id, f32::INFINITY);
    assert_eq!(set.value(id), 1_000.0);
    assert_eq!(set.changes(id), 2);
}

#[test]
fn a_range_starting_at_negative_zero_never_stores_negative_zero() {
    let set = build(&[of("p", "", range(-0.0, 1.0, 0.0, None), f32::NAN)]);
    let id = set.id("p").expect("declared");
    assert_eq!(set.value(id).to_bits(), 0.0_f32.to_bits());
    set.program().load().restore(id, f32::NAN);
    assert_eq!(set.value(id).to_bits(), 0.0_f32.to_bits());
}

#[test]
fn a_reader_that_takes_the_count_first_never_sees_an_older_value() {
    // One writer stores 1, 2, 3, ... and each store moves the
    // count to the same number, so a value below the count it was read
    // after would be an ordering bug.
    const WRITES: u32 = 200_000;
    let set = build(&[of("p", "", range(0.0, WRITES as f32, 1.0, None), 0.0)]);
    let id = set.id("p").expect("declared");
    let started = Arc::new(Latch::new(1));
    let writer = {
        let set = Arc::clone(&set);
        let started = Arc::clone(&started);
        std::thread::spawn(move || {
            started.wait();
            let load = set.program().load();
            for k in 1..=WRITES {
                load.restore(id, k as f32);
            }
        })
    };
    started.count_down();
    let mut seen = 0;
    while seen < u64::from(WRITES) {
        let Reading { changes, value, .. } = set.read(id);
        assert!(
            f64::from(value) >= changes as f64,
            "count {changes}, value {value}"
        );
        assert!(changes >= seen, "the count went back");
        seen = changes;
    }
    writer.join().expect("writer");
    assert_eq!(set.value(id), WRITES as f32);
}

#[test]
fn a_host_id_is_a_fixed_hash_of_the_key() {
    // Hosts save automation against these numbers, so they never
    // change. 32-bit FNV-1a over the key's bytes, the top bit cleared.
    assert_eq!(seam_params::host_id(""), 0x011C_9DC5);
    assert_eq!(seam_params::host_id("a"), 0x640C_292C);
    assert_eq!(seam_params::host_id("gain"), 0x1B54_26FE);
    assert_eq!(seam_params::host_id("hp.freq"), 0x7741_4BE3);
    assert_eq!(seam_params::host_id("out.gain"), 0x492F_4B1C);
    let set = build(&[param("gain", ""), param("hp.freq", "Bands")]);
    for id in set.ids() {
        let host = set.host_id(id);
        assert_eq!(host, seam_params::host_id(set.decl(id).key));
        assert!(host <= 0x7FFF_FFFF, "VST3 ids are 31 bits");
        assert_eq!(set.by_host_id(host), Some(id));
    }
    assert_eq!(set.by_host_id(0x7FFF_0000), None);
}

#[test]
fn a_key_whose_host_id_is_reserved_or_taken_is_refused() {
    // "phgz" hashes into the VST3 exporter's proxy range, and
    // "kmtzx" and "k31cd" share one id.
    assert!(seam_params::RESERVED_HOST_IDS.contains(&seam_params::host_id("phgz")));
    let mut builder = ParamSetBuilder::new();
    assert_eq!(
        builder.add(param("phgz", "")).err(),
        Some(DeclError::ReservedHostId("phgz"))
    );
    builder.add(param("kmtzx", "")).expect("free");
    assert_eq!(
        builder.add(param("k31cd", "")).err(),
        Some(DeclError::HostIdCollision {
            key: "k31cd",
            other: "kmtzx"
        })
    );
    // A refused declaration changed nothing.
    builder.add(param("other", "")).expect("free");
    assert_eq!(builder.build().len(), 2);
}

#[test]
fn the_reserved_range_is_pinned_at_both_edges() {
    // Keys hashing just inside each edge are refused, and one just
    // below the range is accepted.
    assert_eq!(seam_params::RESERVED_HOST_IDS, 0x7FFF_0000..=0x7FFF_FFFF);
    for (key, id) in [("ata7n", 0x7FFF_002E), ("ctdnh", 0x7FFF_FF84)] {
        assert_eq!(seam_params::host_id(key), id);
        assert_eq!(
            ParamSetBuilder::new().add(param(key, "")).err(),
            Some(DeclError::ReservedHostId(key))
        );
    }
    assert_eq!(seam_params::host_id("b8rld"), 0x7FFE_FF07);
    assert!(ParamSetBuilder::new().add(param("b8rld", "")).is_ok());
}
