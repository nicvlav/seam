//! Readings: a change count, a value and its writer's origin, taken
//! together, as a value derived from a parameter, a plain value or a list
//! reads its source.

use seam_params::{Kind, List, Origin, ParamDecl, ParamSet, Value};

#[test]
fn a_plain_value_reads_its_count_its_value_and_that_values_origin_together() {
    let set = ParamSet::from_decls([]).expect("valid");
    let name = Value::new(String::from("Init"), &set);
    let reading = name.read();
    assert_eq!(
        (reading.changes, reading.value.as_str(), reading.origin),
        (0, "Init", Origin::Restore),
        "the initial value is the state it was made with"
    );

    name.set(String::from("Lead"));
    let reading = name.read();
    assert_eq!(
        (reading.changes, reading.value.as_str(), reading.origin),
        (1, "Lead", Origin::User)
    );
    name.set_from(String::from("Pad"), Origin::Host);
    let reading = name.read();
    assert_eq!(
        (reading.changes, reading.value.as_str(), reading.origin),
        (2, "Pad", Origin::Host)
    );
}

#[test]
fn a_list_reads_what_its_closure_makes_with_the_count_and_the_origin() {
    let set = ParamSet::from_decls([]).expect("valid");
    let samples = List::new(vec!["kick"], &set);
    let reading = samples.read(<[_]>::len);
    assert_eq!(
        (reading.changes, reading.value, reading.origin),
        (0, 1, Origin::Restore)
    );

    samples.push("snare");
    let reading = samples.read(<[_]>::len);
    assert_eq!(
        (reading.changes, reading.value, reading.origin),
        (1, 2, Origin::User)
    );
    samples.derived(Origin::Learn).push("hat");
    let reading = samples.read(|items| items.join(", "));
    assert_eq!(
        (reading.changes, reading.value.as_str(), reading.origin),
        (2, "kick, snare, hat", Origin::Learn)
    );
    samples.reset(Vec::new());
    let reading = samples.read(<[_]>::is_empty);
    assert_eq!(
        (reading.changes, reading.value, reading.origin),
        (3, true, Origin::Restore)
    );
}

#[test]
fn a_value_derived_from_a_reading_passes_on_the_origin_of_the_value_it_used() {
    let gain = ParamDecl::new(
        "gain",
        "Gain",
        Kind::continuous(-60.0, 12.0, 0.1, None),
        0.0,
    );
    let set = ParamSet::from_decls([gain]).expect("valid");
    let gain = set.id("gain").expect("declared");
    let shown = Value::new(0.0_f32, &set);

    set.audio().host_write(gain, 1.0);
    let reading = set.read(gain);
    shown.set_from(reading.value * 2.0, reading.origin);
    let derived = shown.read();
    assert_eq!(
        (reading.changes, derived.value, derived.origin),
        (1, 24.0, Origin::Host)
    );
}

#[test]
fn a_reading_taken_while_another_thread_writes_is_never_older_than_its_count() {
    // Each write's value is its own change number, from an origin that
    // alternates with it, so a reading shows whether its value, its count
    // and its origin came apart.
    const WRITES: u64 = 20_000;
    let set = ParamSet::from_decls([]).expect("valid");
    let value = Value::new(0_u64, &set);
    let list = List::new(Vec::<u64>::new(), &set);
    let origin_of = |n: u64| {
        if n.is_multiple_of(2) {
            Origin::Host
        } else {
            Origin::User
        }
    };
    // Reads each once, checks the readings, and returns their counts, which
    // must never go back.
    let check = |last: (u64, u64)| {
        let reading = value.read();
        assert!(
            reading.value >= reading.changes,
            "a value older than its count"
        );
        assert_eq!(
            reading.origin,
            if reading.value == 0 {
                Origin::Restore
            } else {
                origin_of(reading.value)
            }
        );
        let listed = list.read(|items| items.last().copied().unwrap_or(0));
        assert!(
            listed.value >= listed.changes,
            "elements older than their count"
        );
        assert!(
            reading.changes >= last.0 && listed.changes >= last.1,
            "a count went back"
        );
        (reading.changes, listed.changes)
    };
    std::thread::scope(|scope| {
        let writer = scope.spawn(|| {
            for n in 1..=WRITES {
                value.set_from(n, origin_of(n));
                list.derived(origin_of(n)).push(n);
            }
        });
        let mut last = (0, 0);
        // Until the writer ends, so a writer that panics or stops fails the
        // test instead of hanging it.
        while !writer.is_finished() {
            last = check(last);
        }
        writer.join().expect("the writer ran to its end");
        assert_eq!(check(last), (WRITES, WRITES), "every write counted");
    });
}
