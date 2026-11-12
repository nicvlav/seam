//! The rules one declaration can break on its own, each written once as a
//! `const fn`. A typed declaration's `checked` applies them while compiling,
//! and [`ParamSetBuilder::add`](crate::ParamSetBuilder::add) and
//! [`Declarer::add`](crate::Declarer::add) apply the same ones when a
//! parameter is added. What only the whole set shows, a key another
//! parameter has, say, the builder checks on its own.

use crate::{DeclError, MAX_KEY_LEN, RESERVED_HOST_IDS, Range, host_id};

/// `first`, or `then` when `first` passed: a `const fn` cannot use `?`.
pub(crate) const fn and(
    first: Result<(), DeclError>,
    then: Result<(), DeclError>,
) -> Result<(), DeclError> {
    match first {
        Ok(()) => then,
        Err(error) => Err(error),
    }
}

/// The key rules: not empty, no `/`, at most [`MAX_KEY_LEN`] bytes, and a
/// host id outside the range the VST3 exporter reserves.
pub(crate) const fn key(key: &'static str) -> Result<(), DeclError> {
    let bytes = key.as_bytes();
    if bytes.is_empty() {
        return Err(DeclError::EmptyKey);
    }
    if split_name(bytes).1.is_some() {
        return Err(DeclError::KeyHasSlash(key));
    }
    if bytes.len() > MAX_KEY_LEN {
        return Err(DeclError::KeyTooLong(key));
    }
    let id = host_id(key);
    if id >= *RESERVED_HOST_IDS.start() && id <= *RESERVED_HOST_IDS.end() {
        return Err(DeclError::ReservedHostId(key));
    }
    Ok(())
}

/// The group rules: no empty name along the path, and none that is the
/// parameter's own key.
pub(crate) const fn group(key: &'static str, group: &'static str) -> Result<(), DeclError> {
    if group.is_empty() {
        return Ok(());
    }
    let mut rest = group.as_bytes();
    loop {
        let (name, after) = split_name(rest);
        if name.is_empty() {
            return Err(DeclError::EmptyGroupName { key, group });
        }
        if same(name, key.as_bytes()) {
            return Err(DeclError::KeyIsGroup(key));
        }
        match after {
            Some(after) => rest = after,
            None => return Ok(()),
        }
    }
}

/// The label rules: at least two, none empty or padded with whitespace as
/// `str::trim` reads it, and no two that differ only in ASCII case.
pub(crate) const fn labels(
    key: &'static str,
    labels: &'static [&'static str],
) -> Result<(), DeclError> {
    if labels.len() < 2 {
        return Err(DeclError::TooFewChoices(key));
    }
    let mut index = 0;
    while index < labels.len() {
        let (earlier, rest) = labels.split_at(index);
        if let [label, ..] = rest
            && let Err(error) = label_among(key, label, earlier)
        {
            return Err(error);
        }
        index += 1;
    }
    Ok(())
}

/// The rules one label keeps among the labels before it: not empty, not
/// padded with whitespace as `str::trim` reads it, and not one of `earlier`
/// but for ASCII case.
pub(crate) const fn label_among(
    key: &'static str,
    label: &'static str,
    earlier: &[&'static str],
) -> Result<(), DeclError> {
    match whitespace(label) {
        Whitespace::All => return Err(DeclError::EmptyChoiceLabel(key)),
        Whitespace::AtAnEnd => return Err(DeclError::PaddedChoiceLabel { key, label }),
        Whitespace::Inside => {}
    }
    let mut at = 0;
    while at < earlier.len() {
        if earlier[at]
            .as_bytes()
            .eq_ignore_ascii_case(label.as_bytes())
        {
            return Err(DeclError::DuplicateChoiceLabel { key, label });
        }
        at += 1;
    }
    Ok(())
}

/// The range rules, as [`Range::new`] states them, and a default inside the
/// range, its ends included.
pub(crate) const fn range(
    key: &'static str,
    start: f32,
    end: f32,
    step: f32,
    skew_centre: Option<f32>,
    default: f32,
) -> Result<Range, DeclError> {
    let range = match Range::new(start, end, step, skew_centre) {
        Ok(range) => range,
        Err(error) => return Err(DeclError::RangeRefused { key, error }),
    };
    // Written so that a NaN default is outside too.
    if !(default >= start && default <= end) {
        return Err(DeclError::DefaultOutsideRange(key));
    }
    Ok(range)
}

/// Where a label has whitespace, as `str::trim` reads it.
enum Whitespace {
    /// Only whitespace, or nothing at all.
    All,
    /// Some at an end, and something else.
    AtAnEnd,
    /// None at either end.
    Inside,
}

const fn whitespace(text: &str) -> Whitespace {
    let mut rest = text.as_bytes();
    let mut first = None;
    let mut last = false;
    let mut all = true;
    while let Some((char, after)) = split_char(rest) {
        let blank = char.is_whitespace();
        if first.is_none() {
            first = Some(blank);
        }
        last = blank;
        all = all && blank;
        rest = after;
    }
    if all {
        Whitespace::All
    } else if matches!(first, Some(true)) || last {
        Whitespace::AtAnEnd
    } else {
        Whitespace::Inside
    }
}

/// The first char of `bytes`, which are UTF-8, and the bytes after it;
/// `None` at the end.
const fn split_char(bytes: &[u8]) -> Option<(char, &[u8])> {
    let (scalar, rest) = match bytes {
        [] => return None,
        [a, rest @ ..] if *a < 0x80 => (*a as u32, rest),
        [a, b, rest @ ..] if *a < 0xE0 => ((((*a & 0x1F) as u32) << 6) | low(*b), rest),
        [a, b, c, rest @ ..] if *a < 0xF0 => (
            (((*a & 0x0F) as u32) << 12) | (low(*b) << 6) | low(*c),
            rest,
        ),
        [a, b, c, d, rest @ ..] => (
            (((*a & 0x07) as u32) << 18) | (low(*b) << 12) | (low(*c) << 6) | low(*d),
            rest,
        ),
        // A `str` never ends inside a char.
        _ => return None,
    };
    match char::from_u32(scalar) {
        Some(char) => Some((char, rest)),
        None => None,
    }
}

/// The six bits a continuation byte carries.
const fn low(byte: u8) -> u32 {
    (byte & 0x3F) as u32
}

/// The name before the first `/` of `path`, and the path after that slash
/// if there is one.
const fn split_name(path: &[u8]) -> (&[u8], Option<&[u8]>) {
    let mut at = 0;
    while at < path.len() {
        if path[at] == b'/' {
            let (name, slash) = path.split_at(at);
            return match slash.split_first() {
                Some((_, after)) => (name, Some(after)),
                None => (name, None),
            };
        }
        at += 1;
    }
    (path, None)
}

/// Whether `a` and `b` hold the same bytes.
pub(crate) const fn same(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut at = 0;
    while at < a.len() {
        if a[at] != b[at] {
            return false;
        }
        at += 1;
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn whitespace_is_read_as_trim_reads_it() {
        for text in [
            "",
            " ",
            "a",
            " a",
            "a ",
            "a b",
            "\u{a0}a",
            "a\u{3000}",
            "\u{2003}",
            "é",
            "日本",
            "a\u{1F600}",
            "\u{85}",
        ] {
            let expected = if text.trim().is_empty() {
                "all"
            } else if text.trim() != text {
                "at an end"
            } else {
                "inside"
            };
            let found = match whitespace(text) {
                Whitespace::All => "all",
                Whitespace::AtAnEnd => "at an end",
                Whitespace::Inside => "inside",
            };
            assert_eq!(found, expected, "{text:?}");
        }
    }

    #[test]
    fn the_key_length_rule_states_the_longest_key() {
        // The words are written out, since a `const fn` cannot format.
        let rule = DeclError::KeyTooLong("").rule();
        assert!(rule.ends_with(&format!(" {MAX_KEY_LEN} bytes")), "{rule}");
    }

    #[test]
    fn a_char_is_split_off_as_str_chars_reads_it() {
        let text = "a\u{e9}\u{65e5}\u{1F600}/";
        let mut rest = text.as_bytes();
        let mut chars = Vec::new();
        while let Some((char, after)) = split_char(rest) {
            chars.push(char);
            rest = after;
        }
        assert_eq!(chars, text.chars().collect::<Vec<_>>());
    }
}
