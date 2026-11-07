//! The guide `docs/first-plugin.md` quotes this crate: each of its Rust and
//! TOML blocks is a run of lines of `src/lib.rs` or `Cargo.toml`, indentation
//! and blank lines aside, so a change to quoted code fails here until the
//! guide follows it.

use std::path::Path;

fn read(path: &str) -> String {
    let file = Path::new(env!("CARGO_MANIFEST_DIR")).join(path);
    let text = std::fs::read_to_string(&file)
        .unwrap_or_else(|error| panic!("{}: {error}", file.display()));
    text.replace("\r\n", "\n")
}

/// The lines of `text`, trimmed, without the blank ones.
fn lines(text: &str) -> Vec<&str> {
    text.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect()
}

/// The bodies of the guide's blocks fenced as `language`.
fn blocks<'a>(guide: &'a str, language: &str) -> Vec<&'a str> {
    let open = format!("```{language}\n");
    guide
        .split(open.as_str())
        .skip(1)
        .map(|rest| rest.split_once("```").expect("a closed block").0)
        .collect()
}

#[test]
fn every_rust_and_toml_block_in_the_guide_is_quoted_from_the_crate() {
    let guide = read("../../docs/first-plugin.md");
    for (language, path) in [("rust", "src/lib.rs"), ("toml", "Cargo.toml")] {
        let source = read(path);
        let source = lines(&source);
        let quoted = blocks(&guide, language);
        assert!(!quoted.is_empty(), "the guide shows {language}");
        for block in quoted {
            let block_lines = lines(block);
            assert!(!block_lines.is_empty(), "an empty {language} block");
            assert!(
                source
                    .windows(block_lines.len())
                    .any(|run| run == block_lines.as_slice()),
                "the guide's block is not a run of lines of {path}:\n{block}"
            );
        }
    }
}
