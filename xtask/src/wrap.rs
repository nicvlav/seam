//! The prose wrap width: `cargo xtask wrap` lists the `///`, `//!` and `//`
//! lines of `crates/`, `examples/` and `xtask/` that run past it, and
//! `cargo xtask wrap --markdown` the prose lines of the repository's Markdown.
//!
//! The comments come from [`crate::scan`], so a `//` inside a string is not
//! one. Exempt are code in doc examples (a fenced block, or an indented one),
//! table rows, and lines that hold a URL, which cannot be broken. The Markdown
//! mode's exemptions are on [`markdown_overlong`].

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use crate::scan::{CommentKind, scan};

/// The width in columns: 80, which rustfmt uses for comments (its
/// `comment_width`). The repository keeps every prose line within it.
pub const DEFAULT_MAX: usize = 80;

/// The directories, from the repository root, whose Rust comments the width
/// covers.
pub const RUST_DIRS: [&str; 3] = ["crates", "examples", "xtask"];

/// The Markdown files, from the repository root, the Markdown mode covers,
/// besides every `.md` file directly in `docs/`.
pub const MARKDOWN_FILES: [&str; 3] = ["README.md", "ARCHITECTURE.md", "CHANGELOG.md"];

/// The doc whose list lines a script reads whole, so they do not wrap.
const READ_WHOLE: &str = "docs/decision-index.md";

/// One line that runs past the width.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Overlong {
    /// The 1-based source line.
    pub line: usize,
    /// Its length in characters.
    pub columns: usize,
}

/// Returns the prose lines of `source` longer than `max` characters.
///
/// # Errors
///
/// Returns why the source does not lex.
pub fn overlong(source: &str, max: usize) -> Result<Vec<Overlong>, String> {
    let found = scan(source)?;
    let lines: Vec<&str> = source.lines().collect();
    let mut out = Vec::new();
    let mut fence: Option<(usize, CommentKind)> = None;
    for comment in &found.comments {
        let prose = matches!(
            comment.kind,
            CommentKind::Line | CommentKind::OuterDoc | CommentKind::InnerDoc
        );
        if !prose {
            continue;
        }
        let doc = comment.kind != CommentKind::Line;
        let body = comment.text.trim_start_matches('/').trim_start_matches('!');
        let trimmed = body.trim_start();
        // A fence holds only across adjacent lines of one kind.
        if let Some((last, kind)) = fence
            && (comment.line != last + 1 || kind != comment.kind)
        {
            fence = None;
        }
        if doc && trimmed.starts_with("```") {
            fence = match fence {
                Some(_) => None,
                None => Some((comment.line, comment.kind)),
            };
            continue;
        }
        if let Some((_, kind)) = fence {
            fence = Some((comment.line, kind));
            continue;
        }
        let indented_code = doc && body.starts_with("     ");
        let exempt = indented_code || trimmed.starts_with('|') || comment.text.contains("://");
        let columns = lines
            .get(comment.line - 1)
            .map_or(0, |line| line.trim_end_matches('\r').chars().count());
        if !exempt && columns > max {
            out.push(Overlong {
                line: comment.line,
                columns,
            });
        }
    }
    Ok(out)
}

/// Returns the prose lines of the Markdown `source` longer than `max`
/// characters.
///
/// Exempt are fenced code (between two fences of three or more backticks or
/// tildes), headings, table rows, link reference definitions, and a line
/// holding a word or a link too long to fit the width on a line of its own at
/// that line's indent, a long URL say, which no break can shorten. A link,
/// `[text](target)` or `[text][label]`, counts as one word, its text's spaces
/// included, since breaking it would split the link.
pub fn markdown_overlong(source: &str, max: usize) -> Vec<Overlong> {
    let mut out = Vec::new();
    // The open fence's character and length.
    let mut fence: Option<(char, usize)> = None;
    for (index, line) in source.lines().enumerate() {
        let line = line.trim_end_matches('\r');
        let trimmed = line.trim_start();
        let marker = trimmed.chars().next().filter(|c| matches!(c, '`' | '~'));
        let run = marker.map_or(0, |c| trimmed.chars().take_while(|&d| d == c).count());
        match (fence, marker) {
            (None, Some(c)) if run >= 3 => {
                fence = Some((c, run));
                continue;
            }
            (Some((c, length)), Some(d))
                if c == d && run >= length && trimmed[run..].trim().is_empty() =>
            {
                fence = None;
                continue;
            }
            (Some(_), _) => continue,
            _ => {}
        }
        let columns = line.chars().count();
        if columns <= max
            || trimmed.starts_with('#')
            || trimmed.starts_with('|')
            || is_link_definition(trimmed)
            || has_unbreakable_word(line, max)
        {
            continue;
        }
        out.push(Overlong {
            line: index + 1,
            columns,
        });
    }
    out
}

/// Whether `trimmed` is a link reference definition: `[label]: target`.
fn is_link_definition(trimmed: &str) -> bool {
    trimmed.starts_with('[')
        && trimmed
            .find("]:")
            .is_some_and(|end| !trimmed[1..end].contains(']'))
}

/// `text` with the spaces inside each link, `[text](target)` or
/// `[text][label]`, made no-break spaces, so that a link splits as one word.
fn joined_links(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(open) = rest.find('[') {
        out.push_str(&rest[..open]);
        let link = &rest[open..];
        let end = link
            .find(']')
            .filter(|&close| !link[1..close].contains('['));
        let end = end.and_then(|close| {
            let closing = match link[close + 1..].chars().next()? {
                '(' => ')',
                '[' => ']',
                _ => return None,
            };
            let after = link[close + 2..].find(closing)?;
            Some(close + 2 + after + 1)
        });
        match end {
            Some(end) => {
                out.push_str(&link[..end].replace(' ', "\u{a0}"));
                rest = &link[end..];
            }
            None => {
                out.push('[');
                rest = &link[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

/// Whether `line` holds a word that would pass `max` on a line of its own,
/// at the line's indent and after its list or quote marker.
fn has_unbreakable_word(line: &str, max: usize) -> bool {
    let joined = joined_links(line.trim_start());
    let text = joined.as_str();
    let mut indent = line.chars().count() - line.trim_start().chars().count();
    let mut words = text.split(' ').filter(|word| !word.is_empty()).peekable();
    let is_marker = |word: &str| {
        matches!(word, "-" | "*" | "+" | ">")
            || word.strip_suffix(['.', ')']).is_some_and(|number| {
                !number.is_empty() && number.chars().all(|c| c.is_ascii_digit())
            })
    };
    if let Some(marker) = words.next_if(|word| is_marker(word)) {
        indent += marker.chars().count() + 1;
    }
    words.any(|word| indent + word.chars().count() > max)
}

/// Every `.rs` file under `dir`, skipping build output, in path order.
///
/// # Errors
///
/// Returns the I/O error that stopped the walk.
pub fn rust_files(dir: &Path) -> std::io::Result<Vec<PathBuf>> {
    let mut found = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(next) = stack.pop() {
        for entry in std::fs::read_dir(&next)? {
            let path = entry?.path();
            if path.is_dir() {
                if path.file_name().is_some_and(|n| n != "target") {
                    stack.push(path);
                }
            } else if path.extension().is_some_and(|e| e == "rs") {
                found.push(path);
            }
        }
    }
    found.sort();
    Ok(found)
}

/// The Markdown files the Markdown mode reads, in path order.
///
/// # Errors
///
/// Returns the I/O error that stopped the listing of `docs/`.
pub fn markdown_files(root: &Path) -> std::io::Result<Vec<PathBuf>> {
    let mut found: Vec<PathBuf> = MARKDOWN_FILES.iter().map(|name| root.join(name)).collect();
    for entry in std::fs::read_dir(root.join("docs"))? {
        let path = entry?.path();
        if path.is_file() && path.extension().is_some_and(|e| e == "md") {
            found.push(path);
        }
    }
    found.sort();
    Ok(found)
}

/// `path` from `root`, with `/` separators.
fn shown(root: &Path, path: &Path) -> String {
    let relative = path.strip_prefix(root).unwrap_or(path);
    relative.to_string_lossy().replace('\\', "/")
}

/// The package a file of `RUST_DIRS` belongs to: `crates/host`,
/// `examples/eq`, or `xtask`.
fn package_of(shown: &str) -> String {
    let mut parts = shown.split('/');
    match (parts.next(), parts.next()) {
        (Some(top @ ("crates" | "examples")), Some(name)) => format!("{top}/{name}"),
        (Some(top), _) => top.to_string(),
        (None, _) => "?".to_string(),
    }
}

/// Prints each hit, then a count per group, and returns the total.
fn report(counts: &BTreeMap<String, usize>, max: usize) -> usize {
    let total: usize = counts.values().sum();
    for (name, count) in counts {
        println!("{name}: {count} line(s) over {max} columns");
    }
    println!("total: {total}");
    total
}

/// Runs the task over the Rust comments of [`RUST_DIRS`]: prints each
/// overlong line and a count per package. Returns how many lines it found.
///
/// # Errors
///
/// Returns the error that stopped a file from being read, or a directory
/// with no Rust file in it.
pub fn run(root: &Path, max: usize) -> Result<usize, String> {
    let mut counts: BTreeMap<String, usize> = BTreeMap::new();
    for dir in RUST_DIRS {
        let path = root.join(dir);
        let files = rust_files(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        if files.is_empty() {
            return Err(format!(
                "no Rust file under {dir}/; the walk is out of date"
            ));
        }
        for path in files {
            let shown = shown(root, &path);
            let source = std::fs::read_to_string(&path).map_err(|e| format!("{shown}: {e}"))?;
            let count = counts.entry(package_of(&shown)).or_insert(0);
            for hit in overlong(&source, max).map_err(|e| format!("{shown}: {e}"))? {
                println!("{shown}:{}: {} columns", hit.line, hit.columns);
                *count += 1;
            }
        }
    }
    Ok(report(&counts, max))
}

/// Runs the Markdown mode over [`markdown_files`]: prints each overlong
/// prose line and a count per file. Returns how many lines it found. The
/// decision index's list lines are left out, since its check reads each whole.
///
/// # Errors
///
/// Returns the error that stopped a file from being read.
pub fn run_markdown(root: &Path, max: usize) -> Result<usize, String> {
    let files = markdown_files(root).map_err(|e| format!("{}: {e}", root.display()))?;
    let mut counts: BTreeMap<String, usize> = BTreeMap::new();
    for path in files {
        let shown = shown(root, &path);
        let source = std::fs::read_to_string(&path).map_err(|e| format!("{shown}: {e}"))?;
        let lines: Vec<&str> = source.lines().collect();
        let count = counts.entry(shown.clone()).or_insert(0);
        for hit in markdown_overlong(&source, max) {
            let read_whole = shown == READ_WHOLE
                && lines
                    .get(hit.line - 1)
                    .is_some_and(|line| line.starts_with("- "));
            if !read_whole {
                println!("{shown}:{}: {} columns", hit.line, hit.columns);
                *count += 1;
            }
        }
    }
    Ok(report(&counts, max))
}
