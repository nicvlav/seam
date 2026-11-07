//! Host-facing metadata: step counts, discreteness and real-unit text.

use seam_params::{Kind, ParamDecl, fit_text};

const LABELS: [&str; 3] = ["Low", "Mid", "High"];

fn range(start: f32, end: f32, interval: f32) -> Kind {
    Kind::continuous(start, end, interval, None)
}

fn decl(kind: Kind, unit: &'static str) -> ParamDecl {
    ParamDecl::new("p", "P", kind, 0.0).with_unit(unit)
}

#[test]
fn steps_count_every_reachable_value_and_0_means_continuous() {
    assert_eq!(range(0.0, 1.0, 0.0).steps(), 0, "the sentinel");
    // 0, 3, 6, 9 and the off-lattice end 10.
    assert_eq!(range(0.0, 10.0, 3.0).steps(), 5);
    // 0, 3, 6, 9, with the end on the lattice.
    assert_eq!(range(0.0, 9.0, 3.0).steps(), 4);
    assert_eq!(range(-1.0, 1.0, 0.5).steps(), 5);
    assert_eq!(Kind::Bool.steps(), 2);
    assert_eq!(Kind::Choice(&LABELS).steps(), 3);
    // Saturates rather than overflowing.
    assert_eq!(range(0.0, 1.0e9, 1.0e-3).steps(), u32::MAX);
}

#[test]
fn a_step_finer_than_the_f32_spacing_counts_points_that_share_an_f32() {
    // From 2^24 the f32s are 2 apart: 169 points in steps of 0.5 land on the
    // 43 values snapping reaches.
    let kind = range(16_777_216.0, 16_777_300.0, 0.5);
    assert_eq!(kind.steps(), 169);
    let reached: std::collections::BTreeSet<u32> = (0..=168_u16)
        .map(|k| {
            let point = 16_777_216.0 + f32::from(k) * 0.5;
            kind.snap(point).expect("finite").to_bits()
        })
        .collect();
    assert_eq!(reached.len(), 43);
}

#[test]
fn only_bool_and_choice_are_discrete() {
    // A stepped range is continuous to every host.
    assert!(!range(0.0, 10.0, 1.0).is_discrete());
    assert!(Kind::Bool.is_discrete() && Kind::Bool.is_boolean());
    assert!(Kind::Choice(&LABELS).is_discrete());
    assert!(!Kind::Choice(&LABELS).is_boolean());
}

#[test]
fn text_uses_the_steps_decimals() {
    let continuous = decl(range(-1.0, 1.0, 0.0), "");
    assert_eq!(continuous.format_real(0.5), "0.50");
    assert_eq!(continuous.format_real(-0.001), "0.00", "no sign on zero");

    let whole = decl(range(0.0, 10.0, 1.0), "");
    assert_eq!(whole.format_real(7.0), "7");

    let quarters = decl(range(0.0, 1.0, 0.25), "");
    assert_eq!(quarters.format_real(0.5), "0.5");
    assert_eq!(quarters.format_real(0.25), "0.25");
    assert_eq!(quarters.format_real(1.0), "1");

    let fine = decl(range(0.0, 1.0, 0.001), "");
    assert_eq!(fine.format_real(0.123), "0.123");

    // Every place the step has, however few its first significant digit
    // suggests.
    let eighths = decl(range(0.0, 1.0, 0.125), "");
    assert_eq!(eighths.format_real(0.125), "0.125");
    assert_eq!(eighths.format_real(0.375), "0.375");
    assert_eq!(eighths.format_real(0.5), "0.5");
    let eightieths = decl(range(0.0, 1.0, 0.0125), "");
    assert_eq!(eightieths.format_real(0.0125), "0.0125");
    assert_eq!(eightieths.format_real(0.025), "0.025");

    let tiny = decl(range(0.0, 1.0, 1.0e-9), "");
    assert_eq!(tiny.format_real(0.5), "0.5", "at most 6 decimals, trimmed");
}

#[test]
fn text_counts_the_starts_and_the_steps_places_so_every_point_shows_as_itself() {
    // A whole step from 0.5: the points sit on the halves.
    let halves = decl(range(0.5, 10.5, 1.0), "");
    assert_eq!(halves.format_real(0.5), "0.5");
    assert_eq!(halves.format_real(1.5), "1.5");
    assert_eq!(halves.format_real(10.5), "10.5");

    for (start, end, step) in [
        (0.5, 10.5, 1.0),
        (0.25, 5.25, 0.5),
        (-0.125, 9.875, 1.0),
        (0.1, 10.1, 2.0),
        (-60.5, 12.5, 0.1),
        (0.0, 1.0, 0.125),
        (0.0, 1.0, 0.0125),
    ] {
        let param = decl(range(start, end, step), "");
        let mut k = 0.0_f32;
        while start + k * step <= end {
            let point = param.edit_value(start + k * step).expect("finite");
            let text = param.format_real(point);
            // The point's own decimal, not a nearby one the write snaps back.
            assert_eq!(
                text.parse::<f32>(),
                Ok(point),
                "{start}..{end} by {step}: {text}"
            );
            let typed = param.parse_real(&text, 0.0).expect("a number");
            assert_eq!(
                param.edit_value(typed),
                Some(point),
                "{start}..{end} by {step}: {text}"
            );
            k += 1.0;
        }
    }
}

#[test]
fn an_off_lattice_end_shows_text_that_reads_back_as_itself() {
    let param = decl(range(0.0, 10.5, 3.0), "");
    let text = param.format_real(10.5);
    assert_eq!(text, "10.5");
    let typed = param.parse_real(&text, 0.0).expect("a number");
    assert_eq!(param.edit_value(typed), Some(10.5));
    // A lattice point still shows the step's decimals.
    assert_eq!(param.format_real(9.0), "9");
}

#[test]
fn a_number_parses_raw_and_the_write_snaps_it() {
    let param = decl(range(0.0, 10.0, 1.0), "dB");
    assert_eq!(param.parse_real(" 3.4 ", 0.0), Some(3.4));
    assert_eq!(param.parse_real("3.4 dB", 0.0), Some(3.4));
    assert_eq!(param.parse_real("3.4dB", 0.0), Some(3.4));
    assert_eq!(param.parse_real("loud", 0.0), None);
    assert_eq!(param.parse_real("inf", 0.0), None);
    assert_eq!(param.parse_real("NaN", 0.0), None);
}

#[test]
fn a_bool_shows_on_or_off_and_reads_on_true_or_1() {
    let param = decl(Kind::Bool, "");
    assert_eq!(param.format_real(1.0), "On");
    assert_eq!(param.format_real(0.0), "Off");
    for on in ["on", "On", "TRUE", "1"] {
        assert_eq!(param.parse_real(on, 0.0), Some(1.0), "{on}");
    }
    for off in ["off", "false", "0", "maybe"] {
        assert_eq!(param.parse_real(off, 1.0), Some(0.0), "{off}");
    }
}

#[test]
fn a_choice_shows_its_label_and_an_unknown_label_keeps_the_current_one() {
    let param = decl(Kind::Choice(&LABELS), "");
    assert_eq!(param.format_real(1.0), "Mid");
    assert_eq!(param.parse_real("High", 0.0), Some(2.0));
    assert_eq!(param.parse_real("high", 0.0), Some(2.0));
    // A renamed label never jumps to the first choice.
    assert_eq!(param.parse_real("Medium", 1.0), Some(1.0));
}

#[test]
fn a_maximum_length_of_0_or_less_means_unlimited() {
    assert_eq!(fit_text("Mid range", 0), "Mid range", "AU passes 0");
    assert_eq!(fit_text("Mid range", -1), "Mid range");
    assert_eq!(fit_text("Mid range", 3), "Mid");
    assert_eq!(fit_text("Mid", 8), "Mid");
    // Cut on a character boundary: 'é' is two bytes.
    assert_eq!(fit_text("café", 4), "caf");
}
