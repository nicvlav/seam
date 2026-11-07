//! Snapping, the host's 0..1, each origin's fallback, and defaults.

use proptest::prelude::*;
use proptest::test_runner::FileFailurePersistence;
use seam_params::{Kind, ParamDecl, Range};

const LABELS: [&str; 8] = ["a", "b", "c", "d", "e", "f", "g", "h"];

fn stepped(start: f32, end: f32, interval: f32) -> Kind {
    Kind::continuous(start, end, interval, None)
}

fn decl(kind: Kind, default: f32) -> ParamDecl {
    ParamDecl::new("p", "P", kind, default)
}

fn snap(kind: Kind, value: f32) -> f32 {
    kind.snap(value).expect("a finite value snaps")
}

#[test]
fn a_stepped_range_snaps_to_the_nearest_lattice_point_or_its_end() {
    // For {0, 10, step 3} the lattice is 0, 3, 6, 9, and 10 is
    // legal too.
    let kind = stepped(0.0, 10.0, 3.0);
    assert_eq!(snap(kind, 7.4), 6.0);
    assert_eq!(snap(kind, 8.0), 9.0);
    assert_eq!(snap(kind, 10.0), 10.0);
    assert_eq!(snap(kind, 11.0), 10.0);
    assert_eq!(snap(kind, -1.0), 0.0);
    // Ties go to the higher value, the end included.
    assert_eq!(snap(kind, 4.5), 6.0);
    assert_eq!(snap(kind, 9.5), 10.0);
    assert_eq!(snap(kind, 9.4), 9.0);
}

#[test]
fn an_off_lattice_end_stays_reachable_from_every_origin() {
    // In {0, 10, step 3}, writing 10, a host 1.0 and a reload of 10 must land
    // on 10, not on the last lattice point 9.
    let param = decl(stepped(0.0, 10.0, 3.0), 0.0);
    assert_eq!(param.edit_value(10.0), Some(10.0));
    assert_eq!(param.host_value(1.0), 10.0);
    assert_eq!(param.restore_value(10.0), 10.0);
    assert_eq!(param.learned_value(1.0), Some(10.0));
}

#[test]
fn a_continuous_range_clamps_and_keeps_everything_between() {
    let kind = stepped(-1.0, 1.0, 0.0);
    assert_eq!(snap(kind, 0.123_456), 0.123_456);
    assert_eq!(snap(kind, 5.0), 1.0);
    assert_eq!(snap(kind, -5.0), -1.0);
}

#[test]
fn a_bool_is_on_from_one_half() {
    assert_eq!(snap(Kind::Bool, 0.49), 0.0);
    assert_eq!(snap(Kind::Bool, 0.5), 1.0);
    assert_eq!(snap(Kind::Bool, -3.0), 0.0);
    assert_eq!(snap(Kind::Bool, 7.0), 1.0);
}

#[test]
fn a_choice_rounds_half_to_even_and_clamps_without_wrapping() {
    let kind = Kind::Choice(&LABELS[..3]);
    assert_eq!(snap(kind, 0.5), 0.0);
    assert_eq!(snap(kind, 1.5), 2.0);
    assert_eq!(snap(kind, 1.4), 1.0);
    assert_eq!(snap(kind, 2.5), 2.0);
    assert_eq!(snap(kind, -0.6), 0.0);
    // A value between 2^31 and 2^32 must clamp to the last choice, not wrap
    // negative and land on the first.
    assert_eq!(snap(kind, 3.0e9), 2.0);
    assert_eq!(snap(kind, 1.0e30), 2.0);
    assert_eq!(snap(kind, -1.0e30), 0.0);
}

#[test]
fn no_kind_ever_returns_negative_zero() {
    let kinds = [
        stepped(-1.0, 1.0, 0.0),
        stepped(-1.0, 1.0, 0.5),
        Kind::Bool,
        Kind::Choice(&LABELS[..2]),
    ];
    for kind in kinds {
        assert_eq!(snap(kind, -0.0).to_bits(), 0.0_f32.to_bits(), "{kind:?}");
    }
}

#[test]
fn a_value_that_is_not_finite_never_snaps() {
    let kinds = [
        stepped(0.0, 10.0, 3.0),
        Kind::Bool,
        Kind::Choice(&LABELS[..3]),
    ];
    for kind in kinds {
        for value in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
            assert_eq!(kind.snap(value), None, "{kind:?} {value}");
        }
    }
}

#[test]
fn each_origin_falls_back_by_its_own_rule() {
    let param = decl(stepped(0.0, 10.0, 1.0), 4.0);

    // Host: not finite becomes 0, the range start; +inf too. Out of 0..1
    // clamps.
    assert_eq!(param.host_value(f64::NAN), 0.0);
    assert_eq!(param.host_value(f64::INFINITY), 0.0);
    assert_eq!(param.host_value(f64::NEG_INFINITY), 0.0);
    assert_eq!(param.host_value(2.0), 10.0);
    assert_eq!(param.host_value(-1.0), 0.0);
    assert_eq!(param.host_value(0.42), 4.0);

    // User or program: refused.
    for value in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
        assert_eq!(param.edit_value(value), None);
    }
    assert_eq!(param.edit_value(6.6), Some(7.0));

    // Restore: the default.
    for value in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
        assert_eq!(param.restore_value(value), 4.0);
    }
    assert_eq!(param.restore_value(12.0), 10.0);

    // Learned: an infinity lands on an end; NaN keeps the current value.
    assert_eq!(param.learned_value(f64::INFINITY), Some(10.0));
    assert_eq!(param.learned_value(f64::NEG_INFINITY), Some(0.0));
    assert_eq!(param.learned_value(f64::NAN), None);
    assert_eq!(param.learned_value(0.5), Some(5.0));
}

#[test]
fn a_default_is_snapped_and_falls_back_to_the_range_start() {
    assert_eq!(decl(stepped(0.0, 10.0, 3.0), 7.4).default_value(), 6.0);
    assert_eq!(decl(stepped(2.0, 10.0, 0.0), f32::NAN).default_value(), 2.0);
    assert_eq!(
        decl(stepped(2.0, 10.0, 0.0), f32::INFINITY).default_value(),
        2.0
    );
    assert_eq!(decl(Kind::Bool, 0.7).default_value(), 1.0);
    assert_eq!(decl(Kind::Choice(&LABELS[..4]), 9.0).default_value(), 3.0);
    // A restore of a non-finite value takes the snapped default.
    assert_eq!(
        decl(stepped(0.0, 10.0, 3.0), 7.4).restore_value(f32::NAN),
        6.0
    );
}

#[test]
fn a_choice_and_a_bool_map_onto_the_whole_of_0_to_1() {
    let choice = Kind::Choice(&LABELS[..5]);
    assert_eq!(choice.normalize(2.0), 0.5);
    assert_eq!(choice.denormalize(1.0), 4.0);
    assert_eq!(Kind::Bool.normalize(1.0), 1.0);
    // The span is floored at 1, so a degenerate choice cannot divide by 0.
    let one = Kind::Choice(&LABELS[..1]);
    assert_eq!(one.normalize(0.0), 0.0);
    assert_eq!(one.denormalize(1.0), 1.0);
    assert_eq!(snap(one, one.denormalize(1.0)), 0.0);
}

#[test]
fn skew_moves_the_0_to_1_mapping_and_never_the_snapping() {
    let skewed = Kind::continuous(20.0, 20_000.0, 0.0, Some(1_000.0));
    assert!((skewed.normalize(1_000.0) - 0.5).abs() < 1e-6);
    assert!((skewed.denormalize(0.5) - 1_000.0).abs() < 1e-2);
    assert_eq!(skewed.denormalize(0.0), 20.0);
    assert_eq!(skewed.denormalize(1.0), 20_000.0);
    assert_eq!(snap(skewed, 1_000.0), 1_000.0);
}

/// A range `Range::new` accepts: finite, start below end, a non-negative
/// interval, and a skew centre strictly inside when there is one.
fn range() -> impl Strategy<Value = Range> {
    ranges(prop_oneof![Just(0.0_f32), 1.0e-6_f32..1.0])
}

/// The same, always stepped.
fn stepped_range() -> impl Strategy<Value = Range> {
    ranges(1.0e-6_f32..1.0)
}

/// The nearest f32 to `units / 10^places`, read from its decimal text.
fn decimal(units: i64, places: u32) -> f32 {
    format!("{units}e-{places}").parse().expect("a decimal")
}

/// A decimal range with an end that may sit off its lattice: the range, the
/// start, the step and the end in whole units of `10^-places`, and how many
/// whole steps lie below the end.
type Decimal = (Range, i64, i64, u32, i64, i64);

fn decimal_range_with_tail() -> impl Strategy<Value = Decimal> {
    (decimal_range(), 0.0_f64..1.0).prop_map(|((range, s, t, places, steps), tail)| {
        let extra = (tail * (t - 1) as f64) as i64;
        let end = s + steps * t + extra;
        let range = Range::new(range.start(), decimal(end, places), range.interval(), None)
            .expect("a valid range");
        (range, s, t, places, steps, end)
    })
}

/// The nearest legal value to `value`, found by listing every legal value
/// outright (each the `f32` nearest its decimal, read from its text) and
/// scanning for the closest, a tie to the higher. It shares no arithmetic
/// with the lattice the range builds.
fn nearest_legal((_, s, t, places, steps, end): Decimal, value: f32) -> f32 {
    let legal = (0..=steps)
        .map(|k| decimal(s + k * t, places))
        .chain([decimal(end, places)]);
    let mut nearest: Option<(f32, f64)> = None;
    for candidate in legal {
        let distance = (f64::from(candidate) - f64::from(value)).abs();
        if nearest.is_none_or(|(_, best)| distance <= best) {
            nearest = Some((candidate, distance));
        }
    }
    nearest.expect("a lattice has points").0
}

/// A range, and values in and around it: any, the lattice points, and the
/// midpoints between neighbours, where ties are.
fn range_and_value() -> impl Strategy<Value = (Decimal, f32)> {
    decimal_range_with_tail().prop_flat_map(|listed| {
        let (range, s, t, places, steps, _) = listed;
        let margin = (range.end() - range.start()) / 8.0;
        let point = move |k: i64| decimal(s + k * t, places);
        let value = prop_oneof![
            (range.start() - margin)..(range.end() + margin),
            (0..=steps).prop_map(point),
            (0..steps).prop_map(move |k| {
                ((f64::from(point(k)) + f64::from(point(k + 1))) / 2.0) as f32
            }),
        ];
        (Just(listed), value)
    })
}

/// A range written in decimals: a start and a step with up to three places,
/// and its end `steps` steps on. Returns the range, the start and step in
/// units of the finer place, and that place.
fn decimal_range() -> impl Strategy<Value = (Range, i64, i64, u32, i64)> {
    (
        -100_000_i64..100_000,
        0_u32..=3,
        1_i64..1_000,
        0_u32..=3,
        1_i64..500,
    )
        .prop_map(
            |(start_units, start_places, step_units, step_places, steps)| {
                let places = start_places.max(step_places);
                let s = start_units * 10_i64.pow(places - start_places);
                let t = step_units * 10_i64.pow(places - step_places);
                let range = Range::new(
                    decimal(s, places),
                    decimal(s + steps * t, places),
                    decimal(t, places),
                    None,
                )
                .expect("a valid range");
                (range, s, t, places, steps)
            },
        )
}

/// `step` is the interval as a fraction of the span.
fn ranges(step: impl Strategy<Value = f32>) -> impl Strategy<Value = Range> {
    (
        -1.0e6_f32..1.0e6,
        1.0e-3_f32..1.0e6,
        step,
        proptest::option::of(0.01_f64..0.99),
    )
        .prop_filter_map("a range with room in it", |(start, span, step, centre)| {
            Range::new(
                start,
                start + span,
                step * span,
                centre.map(|c| (f64::from(start) + f64::from(span) * c) as f32),
            )
            .ok()
        })
}

fn kind() -> impl Strategy<Value = Kind> {
    prop_oneof![
        range().prop_map(Kind::Continuous),
        Just(Kind::Bool),
        (2_usize..=LABELS.len()).prop_map(|n| Kind::Choice(&LABELS[..n])),
    ]
}

fn finite() -> impl Strategy<Value = f32> {
    use proptest::num::f32::{NORMAL, SUBNORMAL, ZERO};
    prop_oneof![
        -2.0e6_f32..2.0e6,
        -20.0_f32..20.0,
        NORMAL | SUBNORMAL | ZERO
    ]
}

proptest! {
    // Integration tests have no lib.rs for proptest to sit a regressions
    // folder beside, so name the file.
    #![proptest_config(ProptestConfig {
        failure_persistence: Some(Box::new(FileFailurePersistence::Direct(
            "tests/snap.proptest-regressions",
        ))),
        ..ProptestConfig::default()
    })]

    /// Snapping is idempotent: a value snapped twice equals the value
    /// snapped once, for every kind. A snapped value is also inside the
    /// kind's range, and never -0.
    #[test]
    fn snapping_twice_equals_snapping_once(kind in kind(), value in finite()) {
        let once = kind.snap(value).expect("a finite value snaps");
        let twice = kind.snap(once).expect("a snapped value snaps");
        prop_assert_eq!(once.to_bits(), twice.to_bits());
        prop_assert!(once >= kind.start() && once <= kind.end());
        prop_assert_ne!(once.to_bits(), (-0.0_f32).to_bits());
    }

    /// Both ends stay reachable from a host's 0 and 1.
    #[test]
    fn a_host_reaches_both_ends(kind in kind()) {
        prop_assert_eq!(kind.snap(kind.denormalize(0.0)), Some(kind.start()));
        prop_assert_eq!(kind.snap(kind.denormalize(1.0)), Some(kind.end()));
    }

    /// Snapping is idempotent again, with values drawn inside and just
    /// around a stepped range, where most of the snapping happens.
    #[test]
    fn snapping_inside_a_stepped_range_is_idempotent(
        (range, value) in stepped_range().prop_flat_map(|range| {
            let margin = (range.end() - range.start()) / 8.0;
            (Just(range), (range.start() - margin)..(range.end() + margin))
        })
    ) {
        let kind = Kind::Continuous(range);
        let once = kind.snap(value).expect("a finite value snaps");
        prop_assert_eq!(kind.snap(once).map(f32::to_bits), Some(once.to_bits()));
        prop_assert!(once >= range.start() && once <= range.end());
    }
}

proptest! {
    #![proptest_config(ProptestConfig {
        failure_persistence: Some(Box::new(FileFailurePersistence::Direct(
            "tests/snap.proptest-regressions",
        ))),
        ..ProptestConfig::default()
    })]

    /// Every point of a range written in decimals is the nearest f32
    /// to its decimal, and snaps to itself; a 0 on the lattice is +0.
    #[test]
    fn every_decimal_point_lands_on_its_nearest_f32(
        (range, s, t, places, steps) in decimal_range(),
        pick in 0.0_f64..=1.0,
    ) {
        let kind = Kind::Continuous(range);
        let k = (pick * steps as f64).round() as i64;
        for k in [0, k, steps] {
            let point = decimal(s + k * t, places);
            prop_assert_eq!(kind.snap(point).map(f32::to_bits), Some(point.to_bits()), "point {}", k);
        }
        if s <= 0 && s + steps * t >= 0 && s % t == 0 {
            prop_assert_eq!(kind.snap(0.0).map(f32::to_bits), Some(0.0_f32.to_bits()));
        }
    }

    /// Snapping gives the nearest legal value, as listing them all finds
    /// it.
    #[test]
    fn snapping_gives_the_nearest_legal_value((decimal, value) in range_and_value()) {
        let kind = Kind::Continuous(decimal.0);
        let snapped = kind.snap(value).expect("a finite value snaps");
        let expected = nearest_legal(decimal, value);
        prop_assert_eq!(
            snapped.to_bits(), expected.to_bits(),
            "{} snapped to {} where {} is nearest", value, snapped, expected
        );
    }

    /// The step count a host is told is the number of values snapping
    /// reaches.
    #[test]
    fn the_step_count_is_the_number_of_values_snapping_reaches(
        (range, s, t, places, steps) in decimal_range(),
    ) {
        let magnitude = range.start().abs().max(range.end().abs());
        prop_assume!(range.interval() > magnitude * 1.0e-5);
        let kind = Kind::Continuous(range);
        let reached: std::collections::BTreeSet<u32> = (0..=steps)
            .map(|k| decimal(s + k * t, places))
            .chain([range.end()])
            .map(|value| kind.snap(value).expect("finite").to_bits())
            .collect();
        prop_assert_eq!(kind.steps() as usize, reached.len());
    }
}

#[test]
fn the_step_count_follows_the_decimal_lattice() {
    // A lattice laid out from the f32 step would reach a 3,482nd value that
    // snapping does not.
    let kind = Kind::continuous(-60.0, -42.6, 0.005, None);
    assert_eq!(kind.steps(), 3_481);
}

#[test]
fn a_step_of_six_or_seven_places_lands_on_its_decimals() {
    for (interval, value) in [(0.000_001_f32, 0.123_456_f32), (0.000_000_1, 0.123_456_7)] {
        let kind = Kind::continuous(0.0, 1.0, interval, None);
        assert_eq!(kind.snap(value), Some(value), "{interval}");
    }
}

#[test]
fn a_decimal_step_lands_on_its_decimals() {
    // 0.1f32 is 0.100000001490116, and a lattice laid out from it put
    // 0 dB on a range from -60 at 8.94e-7.
    let gain = Kind::continuous(-60.0, 12.0, 0.1, None);
    assert_eq!(gain.snap(0.0), Some(0.0));
    assert_eq!(gain.snap(0.04), Some(0.0));
    assert_eq!(gain.snap(-3.0), Some(-3.0));
    assert_eq!(gain.snap(7.5), Some(7.5));
    assert_eq!(gain.snap(-0.3), Some(-0.3));
    let fine = Kind::continuous(0.1, 30.0, 0.01, None);
    assert_eq!(fine.snap(1.0), Some(1.0));
    assert_eq!(fine.snap(0.707), Some(0.71));
    // -0.3 + 3 * 0.1 is 5.55e-17 in f64; counted in tenths it is 0.
    let small = Kind::continuous(-0.3, 0.3, 0.1, None);
    assert_eq!(small.snap(0.0).map(f32::to_bits), Some(0.0_f32.to_bits()));
    assert_eq!(small.snap(-0.1), Some(-0.1));
}
