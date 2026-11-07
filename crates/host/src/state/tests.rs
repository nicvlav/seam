use super::*;

#[test]
fn only_numbers_past_f64s_range_are_rewritten_and_never_inside_a_string() {
    let text = br#"[1e400, -1E+309, 1e308, "1e400", "a \" 1e400", 0, -0.5e-999]"#;
    assert_eq!(
        std::str::from_utf8(&finite_numbers(text)).expect("UTF-8"),
        r#"[null, null, 1e308, "1e400", "a \" 1e400", 0, -0.5e-999]"#
    );
    let plain = br#"{"a": 1, "b": "x"}"#;
    assert!(
        matches!(finite_numbers(plain), Cow::Borrowed(_)),
        "unchanged, uncopied"
    );
}

#[test]
fn a_token_json_does_not_allow_as_a_number_is_left_for_the_parser_to_refuse() {
    let text = br#"[01e400, 1.e400, -01e400, 1e+, 1e400.5, 1e400]"#;
    assert_eq!(
        std::str::from_utf8(&finite_numbers(text)).expect("UTF-8"),
        r#"[01e400, 1.e400, -01e400, 1e+, 1e400.5, null]"#
    );
}

#[test]
fn json_numbers_are_told_from_other_tokens() {
    for number in [
        "0", "-0", "7", "-12", "0.5", "1e400", "1E+9", "2.5e-3", "-0.0e0",
    ] {
        assert!(is_json_number(number.as_bytes()), "{number}");
    }
    for token in [
        "", "-", "01", "-01", "1.", "1.e5", ".5", "1e", "1e+", "1-", "1.5.5", "+1",
    ] {
        assert!(!is_json_number(token.as_bytes()), "{token}");
    }
}

#[test]
fn a_declared_value_takes_at_most_value_bytes() {
    // Every exponent with a spread of mantissas, both signs; the longest is
    // a value of 17 digits with four leading zeros, which the bound is.
    let mut longest = 0;
    for exponent in 0..255_u32 {
        for mantissa in [0, 1, 0x2A_AAAA, 0x55_5555, 0x12_3457, 0x7F_FFFE, 0x7F_FFFF] {
            for sign in [0, 1 << 31] {
                let value = f32::from_bits(sign | exponent << 23 | mantissa);
                let bytes = Json::from(f64::from(value)).to_string().len();
                assert!(bytes <= VALUE_BYTES, "{value:e} takes {bytes}");
                longest = longest.max(bytes);
            }
        }
    }
    assert_eq!(longest, VALUE_BYTES, "the bound is tight");
    assert!(Json::Bool(false).to_string().len() <= VALUE_BYTES);
    assert!(Json::from(u64::from(u32::MAX)).to_string().len() <= VALUE_BYTES);
}
