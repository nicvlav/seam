//! The guide `docs/first-plugin.md` quotes this crate: each of its Rust and
//! TOML blocks is a run of lines of `src/lib.rs`, `src/page.rs`,
//! `tests/gain.rs` or `Cargo.toml`, indentation and blank lines aside, so a
//! change to quoted code fails here until the guide follows it.

use std::path::Path;

fn read(path: &str) -> String {
    let file = Path::new(env!("CARGO_MANIFEST_DIR")).join(path);
    let text = std::fs::read_to_string(&file)
        .unwrap_or_else(|error| panic!("{}: {error}", file.display()));
    text.replace("\r\n", "\n")
}

/// The lines of `text`, trimmed, without the blank ones.
fn lines<'a>(text: impl IntoIterator<Item = &'a str>) -> Vec<&'a str> {
    text.into_iter()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect()
}

/// What a block holds, as the first word of its fence's info string says:
/// `rust,ignore` and `rs` are Rust, `toml` is TOML.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Language {
    Rust,
    Toml,
}

impl Language {
    fn of(info: &str) -> Option<Language> {
        let word = info.split([',', ' ', '\t', '{']).next().unwrap_or("");
        match word.to_ascii_lowercase().as_str() {
            "rust" | "rs" => Some(Language::Rust),
            "toml" => Some(Language::Toml),
            _ => None,
        }
    }

    /// The files a block may quote, from the crate's root.
    fn sources(self) -> &'static [&'static str] {
        match self {
            Language::Rust => &["src/lib.rs", "src/page.rs", "tests/gain.rs"],
            Language::Toml => &["Cargo.toml"],
        }
    }
}

/// A line that opens or closes a fenced block: three or more backticks or
/// tildes, then the info string.
struct Fence<'a> {
    marker: char,
    length: usize,
    info: &'a str,
}

impl<'a> Fence<'a> {
    fn of(line: &'a str) -> Option<Fence<'a>> {
        let trimmed = line.trim_start();
        let marker = trimmed.chars().next().filter(|c| matches!(c, '`' | '~'))?;
        let length = trimmed.chars().take_while(|&c| c == marker).count();
        (length >= 3).then(|| Fence {
            marker,
            length,
            info: trimmed[length..].trim(),
        })
    }

    /// Whether it closes `open`: the same marker, at least as many, and no
    /// info string.
    fn closes(&self, open: &Fence<'_>) -> bool {
        self.marker == open.marker && self.length >= open.length && self.info.is_empty()
    }
}

/// A block the guide checks: the line its fence opens on, its language and
/// its body.
struct Block<'a> {
    line: usize,
    language: Language,
    body: Vec<&'a str>,
}

/// The guide's blocks in a language it checks.
fn blocks(guide: &str) -> Vec<Block<'_>> {
    let mut found = Vec::new();
    let mut open: Option<(usize, Fence<'_>, Vec<&str>)> = None;
    for (index, line) in guide.lines().enumerate() {
        let fence = Fence::of(line);
        let closes = match (&open, &fence) {
            (Some((_, opening, _)), Some(fence)) => fence.closes(opening),
            _ => false,
        };
        if closes {
            if let Some((line, opening, body)) = open.take()
                && let Some(language) = Language::of(opening.info)
            {
                found.push(Block {
                    line,
                    language,
                    body,
                });
            }
        } else if let Some((_, _, body)) = &mut open {
            body.push(line);
        } else if let Some(fence) = fence {
            open = Some((index + 1, fence, Vec::new()));
        }
    }
    assert!(open.is_none(), "a block left open");
    found
}

/// Every block of `guide` that is not a run of lines of one of the texts
/// `sources` gives for its language, each named by the line its fence opens
/// on.
fn drifted(guide: &str, sources: impl Fn(Language) -> Vec<String>) -> Vec<String> {
    blocks(guide)
        .into_iter()
        .filter(|block| {
            let quoted = lines(block.body.iter().copied());
            let quotes = |source: &String| {
                let source = lines(source.lines());
                source
                    .windows(quoted.len())
                    .any(|run| run == quoted.as_slice())
            };
            quoted.is_empty() || !sources(block.language).iter().any(quotes)
        })
        .map(|block| format!("line {}:\n{}", block.line, block.body.join("\n")))
        .collect()
}

#[test]
fn every_rust_and_toml_block_in_the_guide_is_quoted_from_the_crate() {
    let guide = read("../../docs/first-plugin.md");
    let shown = blocks(&guide);
    for language in [Language::Rust, Language::Toml] {
        assert!(
            shown.iter().any(|block| block.language == language),
            "the guide shows {language:?}"
        );
    }
    let sources = |language: Language| language.sources().iter().map(|path| read(path)).collect();
    let drifted = drifted(&guide, sources);
    assert!(
        drifted.is_empty(),
        "{} of the guide's blocks quote no run of lines of {:?} or {:?}:\n\n{}",
        drifted.len(),
        Language::Rust.sources(),
        Language::Toml.sources(),
        drifted.join("\n\n")
    );
}

#[test]
fn a_fence_whose_info_string_starts_with_rust_rs_or_toml_is_checked() {
    let guide = "```rust,ignore\nmissing();\n```\n\n~~~rs\nmissing();\n~~~\n\n\
                 ```toml {title}\nmissing = 1\n```\n\n```sh\nmissing\n```\n";
    let sources = |_| vec!["kept();\nkept = 1\n".to_owned()];
    assert_eq!(
        drifted(guide, sources).len(),
        3,
        "every block but the shell"
    );
}

#[test]
fn every_drifted_block_is_named_not_only_the_first() {
    let guide = "```rust\nfirst();\n```\n\n```rust\nkept();\n```\n\n```rust\nlast();\n```\n";
    let sources = |_| vec!["kept();\n".to_owned()];
    let drifted = drifted(guide, sources);
    assert_eq!(drifted.len(), 2, "{drifted:?}");
    assert!(drifted[0].starts_with("line 1:"), "{}", drifted[0]);
    assert!(drifted[1].starts_with("line 9:"), "{}", drifted[1]);
}
