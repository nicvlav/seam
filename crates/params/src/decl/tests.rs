//! Decimal places, lattices and ranges, from inside the module.

use super::*;

#[test]
fn a_value_reads_back_as_the_decimal_it_was_written_as() {
    // Up to nine places, then the f32 itself.
    for (value, decimal) in [
        (0.1_f32, 0.1),
        (0.001, 0.001),
        (1e-6, 1e-6),
        (1e-9, 1e-9),
        (0.123_456_79, 0.123_456_79),
        (-60.0, -60.0),
        (33_554_752.0, 33_554_752.0),
        (f32::MAX, f64::from(f32::MAX)),
    ] {
        let (read, _) = shortest_decimal(value);
        assert_eq!(read, decimal, "{value}");
    }
    assert_eq!(shortest_decimal(0.1).1, Some(1));
    assert_eq!(shortest_decimal(0.001).1, Some(3));
    assert_eq!(shortest_decimal(1e-9).1, Some(9));
    assert_eq!(shortest_decimal(0.123_456_79).1, Some(8));
    assert_eq!(shortest_decimal(-60.0).1, Some(0));
    // More than nine places: the f32 itself.
    for value in [1.234_567_9e-5_f32, 1e-10] {
        assert_eq!(shortest_decimal(value), (f64::from(value), None), "{value}");
    }
    let (zero, places) = shortest_decimal(-0.0);
    assert!(zero == 0.0 && zero.is_sign_negative() && places == Some(0));
}

#[test]
fn a_lattice_counts_in_its_finest_decimal_place() {
    // -0.3 + 3 * 0.1 is not 0 in f64; -3 + 3 * 1 tenths is.
    let range = Range::new(-0.3, 0.3, 0.1, None).expect("legal");
    let lattice = range.lattice().expect("stepped");
    assert_eq!(lattice.last, 6.0);
    assert_eq!(lattice.point(3.0), 0.0);
    assert_eq!(lattice.point(3.0).to_bits(), 0.0_f32.to_bits());
    assert_eq!(lattice.point(1.0), -0.2);
    assert_eq!(lattice.below(-0.05), 2.0);
    assert_eq!(lattice.below(-1.0), 0.0, "never below the first point");
    assert_eq!(lattice.below(9.0), 6.0, "never past the last");
}

#[test]
fn a_range_made_in_a_const_holds_the_lattice_a_range_made_at_run_time_does() {
    // Snapping reads the lattice the range was made with, so the const
    // evaluator and the compiled code must lay out the same one.
    use std::hint::black_box;
    const KINDS: [Kind; 4] = [
        Kind::continuous(-60.0, 12.0, 0.1, None),
        Kind::continuous(20.0, 20_000.0, 1.0, Some(1_000.0)),
        Kind::continuous(-0.3, 0.3, 0.1, None),
        Kind::continuous(0.0, 1.0, 1.234_567_9e-5, None),
    ];
    for kind in KINDS {
        let Kind::Continuous(made) = kind else {
            panic!("a range");
        };
        let again = Range::new(
            black_box(made.start()),
            black_box(made.end()),
            black_box(made.interval()),
            black_box(made.skew_centre()),
        )
        .expect("legal");
        assert!(made.lattice().is_some());
        assert_eq!(made, again);
    }
}

#[test]
fn a_range_from_minus_0_starts_at_0() {
    // A write at the bottom stores the start, so a -0 start would show a sign.
    for interval in [0.0, 0.25] {
        let range = Range::new(-0.0, 1.0, interval, None).expect("legal");
        assert_eq!(range.start().to_bits(), 0.0_f32.to_bits(), "{interval}");
        assert_eq!(range.snap(-1.0).to_bits(), 0.0_f32.to_bits(), "{interval}");
    }
}
