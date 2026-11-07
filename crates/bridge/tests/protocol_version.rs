//! A message that changes shape raises `PROTOCOL_VERSION`.
//!
//! The shapes' fingerprint of every version is committed in
//! `protocol-shape.txt`, one line per version, so a changed shape under the
//! same version fails here, and the way to make it pass is to raise the
//! version and append a line. This test reads only the last line;
//! `scripts/check-shape-append-only.py` runs on pull requests and fails when
//! an earlier line was changed or removed.

use std::path::Path;

use seam_bridge::PROTOCOL_VERSION;
use seam_bridge::typescript::shape_hash;

const FILE: &str = "protocol-shape.txt";

/// The committed lines, `version <n> shape <hash>`, in file order.
fn committed() -> Vec<(u32, String)> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join(FILE);
    let text = std::fs::read_to_string(&path).expect("crates/bridge/protocol-shape.txt");
    parse(&text)
}

fn parse(text: &str) -> Vec<(u32, String)> {
    text.lines()
        .filter(|line| !line.trim().is_empty() && !line.starts_with('#'))
        .map(|line| {
            let words: Vec<&str> = line.split_whitespace().collect();
            let ["version", version, "shape", shape] = words[..] else {
                panic!("{FILE}: `{line}` is not `version <n> shape <hash>`");
            };
            (version.parse().expect("a version number"), shape.to_owned())
        })
        .collect()
}

/// Whether `history`, the current version and the current shape agree; the
/// error says what to do.
fn judge(history: &[(u32, String)], version: u32, now: &str) -> Result<(), String> {
    let Some((last, shape)) = history.last() else {
        return Err(format!(
            "{FILE} has no line; append `version {version} shape {now}`"
        ));
    };
    if history.windows(2).any(|pair| pair[1].0 != pair[0].0 + 1) {
        return Err(format!(
            "{FILE}'s versions are not consecutive: a line was removed or edited"
        ));
    }
    if version < *last {
        return Err(format!(
            "PROTOCOL_VERSION is {version} but {FILE} records version {last}: the version went down"
        ));
    }
    if version > *last {
        let line = format!("version {version} shape {now}");
        return if version == last + 1 {
            Err(format!(
                "PROTOCOL_VERSION is {version}: append the line `{line}` to {FILE}"
            ))
        } else {
            Err(format!(
                "PROTOCOL_VERSION jumped from {last} to {version}: raise it one at a time, appending `version {} shape ...` for each",
                last + 1
            ))
        };
    }
    if shape != now {
        let next = version + 1;
        return Err(format!(
            "a bridge message changed shape but PROTOCOL_VERSION is still {version}.\n\
             A page built before this change would decode wrongly. Raise PROTOCOL_VERSION to \
             {next} in crates/bridge/src/protocol.rs, then append this line to {FILE} (keep the \
             line for version {version} as it is):\n\nversion {next} shape {now}\n\n\
             If only a comment or the formatting changed, nothing changes the hash: look for a \
             field, variant or serde attribute you touched."
        ));
    }
    Ok(())
}

fn line(version: u32, shape: &str) -> (u32, String) {
    (version, shape.to_owned())
}

#[test]
fn the_message_shapes_and_the_protocol_version_change_together() {
    if let Err(problem) = judge(&committed(), PROTOCOL_VERSION, &shape_hash()) {
        panic!("{problem}\nthen run `cargo xtask gen`.");
    }
}

#[test]
fn a_changed_shape_under_the_same_version_asks_for_the_next_version() {
    let history = [line(2, "aaaa")];
    let problem = judge(&history, 2, "bbbb").unwrap_err();
    assert!(problem.contains("Raise PROTOCOL_VERSION to 3"), "{problem}");
    assert!(problem.contains("version 3 shape bbbb"), "{problem}");
    assert!(
        !problem.contains("version 2 shape bbbb"),
        "pasting the offered text must not leave the version unchanged: {problem}"
    );
}

#[test]
fn a_raised_version_needs_its_line_and_then_passes() {
    let history = [line(2, "aaaa")];
    let problem = judge(&history, 3, "bbbb").unwrap_err();
    assert!(problem.contains("version 3 shape bbbb"), "{problem}");
    assert_eq!(
        judge(&[line(2, "aaaa"), line(3, "bbbb")], 3, "bbbb"),
        Ok(())
    );
}

#[test]
fn the_current_shape_under_the_current_version_passes() {
    assert_eq!(judge(&[line(2, "aaaa")], 2, "aaaa"), Ok(()));
}

#[test]
fn a_changed_shape_under_the_same_version_is_offered_the_next_version() {
    // The offer keeps the recorded line and asks for the next version, so
    // pasting a new hash over the old one is not what the failure suggests.
    let problem = judge(&[line(2, "aaaa")], 2, "bbbb").unwrap_err();
    assert!(
        problem.contains("keep the line for version 2 as it is"),
        "{problem}"
    );
}

#[test]
fn a_gap_or_a_lower_version_fails() {
    assert!(judge(&[line(2, "a"), line(4, "b")], 4, "b").is_err());
    assert!(judge(&[line(2, "a")], 4, "b").is_err());
    assert!(judge(&[line(2, "a"), line(3, "b")], 2, "a").is_err());
    assert!(judge(&[], 2, "a").is_err());
}

#[test]
fn the_file_parses_one_line_per_version() {
    let text = "# comment\n\nversion 2 shape abc\nversion 3 shape def\n";
    assert_eq!(parse(text), [line(2, "abc"), line(3, "def")]);
}
