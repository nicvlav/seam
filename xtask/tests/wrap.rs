//! The prose wrap check refuses a long prose line and passes what is exempt.

use xtask::wrap::overlong;

fn lines(source: &str, max: usize) -> Vec<usize> {
    overlong(source, max)
        .expect("the source lexes")
        .into_iter()
        .map(|o| o.line)
        .collect()
}

#[test]
fn a_long_doc_or_comment_line_is_flagged() {
    let source =
        "/// aaaaaaaaaaaaaaaaaaaa\n//! bbbbbbbbbbbbbbbbbbbb\n// cccccccccccccccccccc\n/// ok\n";
    assert_eq!(lines(source, 20), [1, 2, 3]);
}

#[test]
fn a_line_at_the_width_passes() {
    assert_eq!(lines("/// 0123456\n", 11), Vec::<usize>::new());
    assert_eq!(lines("/// 0123456\n", 10), [1]);
}

#[test]
fn width_counts_characters_not_bytes() {
    // Ten characters, sixteen bytes.
    let source = "/// \u{e9}\u{e9}\u{e9}\u{e9}\u{e9}\u{e9}\n";
    assert_eq!(lines(source, 10), Vec::<usize>::new());
    assert_eq!(lines(source, 9), [1]);
}

#[test]
fn code_is_not_prose() {
    let source = "let a_long_name = another_long_name + third_long_name;\n";
    assert_eq!(lines(source, 10), Vec::<usize>::new());
}

#[test]
fn a_slash_pair_in_a_string_is_not_a_comment() {
    let source = "let a = \"// aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\";\n";
    assert_eq!(lines(source, 10), Vec::<usize>::new());
}

#[test]
fn a_fenced_example_is_exempt() {
    let source = "/// Prose.\n/// ```\n/// let very_long_example_line_of_code = 1;\n/// ```\n/// Long prose after the fence.\n";
    assert_eq!(lines(source, 30), [5]);
}

#[test]
fn a_fence_does_not_outlive_its_comment() {
    let source = "/// ```\n/// code\n\n/// long prose after a gap, long prose\n";
    assert_eq!(lines(source, 20), [4]);
}

#[test]
fn tables_urls_and_indented_code_are_exempt() {
    let source = "/// | a very long | table row |\n/// see https://example.org/a/very/long/address\n///     indented code that runs long\n/// long prose that runs past it\n";
    assert_eq!(lines(source, 25), [4]);
}

#[test]
fn a_block_comment_is_not_checked() {
    let source = "/* aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa */\n";
    assert_eq!(lines(source, 10), Vec::<usize>::new());
}

#[test]
fn a_trailing_comment_counts_the_whole_line() {
    assert_eq!(lines("let a = 1; // trailing words\n", 20), [1]);
}

#[test]
fn run_walks_the_crates_examples_and_xtask_and_fails_on_a_long_line() {
    let root = xtask::root();
    // A width no line reaches passes; a width of one fails for the many prose
    // lines.
    assert!(xtask::wrap::run(&root, 100_000).expect("the Rust reads"));
    assert!(!xtask::wrap::run(&root, 1).expect("the Rust reads"));
}

fn markdown(source: &str, max: usize) -> Vec<usize> {
    xtask::wrap::markdown_overlong(source, max)
        .into_iter()
        .map(|o| o.line)
        .collect()
}

#[test]
fn a_long_markdown_prose_line_is_flagged_and_one_at_the_width_passes() {
    let source = "aaaa bbbb cccc\naaaa bbbb\n> quoted prose runs long\n";
    assert_eq!(markdown(source, 9), [1, 3]);
    assert_eq!(markdown(source, 24), Vec::<usize>::new());
}

#[test]
fn fenced_code_headings_and_tables_are_exempt_in_markdown() {
    let source = "# a long heading line\n```rust\nlet a_long_line = 1;\n```\n\
                  ~~~~\n```\nstill code, long\n~~~~\n| a | long | table |\nlong prose here\n";
    assert_eq!(markdown(source, 10), [10]);
}

#[test]
fn a_shorter_or_other_fence_does_not_close_one() {
    let source = "````\n```\nstill inside the fence\n~~~~\n````\nlong prose after\n";
    assert_eq!(markdown(source, 10), [6]);
}

#[test]
fn a_link_definition_and_a_word_no_line_could_hold_are_exempt() {
    let url = "https://example.org/a/very/long/address/that/cannot/break";
    let source = format!(
        "[label]: {url}\nsee {url}\n- [x]({url})\n- see [the link](short), whose words each fit\n"
    );
    // Lines 2 and 3 hold a word longer than 40; line 4's words each fit.
    assert_eq!(markdown(&source, 40), [4]);
    // A definition stays on one line even when its words would each fit.
    let source = "[a-rather-long-label-name]: https://example.org/short\n";
    assert_eq!(markdown(source, 40), Vec::<usize>::new());
}

#[test]
fn a_link_counts_as_one_word_its_text_included() {
    let link = format!("[two words]({})", "t".repeat(30));
    let source = format!("{link}\nsee {link}\n[a](b) and [c][d] fit, then more words follow it\n");
    // The link is 43 columns, past 40 however its text breaks.
    assert_eq!(markdown(&source, 40), [3]);
}

#[test]
fn a_list_marker_counts_towards_a_words_room() {
    let word = "w".repeat(39);
    // Two columns of marker: the word fits alone at 41, not at 40, where no
    // break would help.
    let source = format!("- {word} x\n");
    assert_eq!(markdown(&source, 40), Vec::<usize>::new());
    assert_eq!(markdown(&source, 41), [1]);
    assert_eq!(markdown(&source, 43), Vec::<usize>::new());
}

/// A temporary repository root, removed on drop.
struct Root(std::path::PathBuf);

impl Root {
    /// A root holding one Rust file in each of `RUST_DIRS`, whose comment is
    /// long only in `long`.
    fn rust(long: &str) -> Root {
        let root =
            std::env::temp_dir().join(format!("seam-wrap-rust-{long}-{}", std::process::id()));
        for dir in xtask::wrap::RUST_DIRS {
            let words = if dir == long {
                "long words, long words"
            } else {
                "short"
            };
            std::fs::create_dir_all(root.join(dir).join("src")).expect("a temporary directory");
            std::fs::write(root.join(dir).join("src/lib.rs"), format!("// {words}\n"))
                .expect("written");
        }
        Root(root)
    }
}

impl Drop for Root {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn run_reads_the_comments_of_every_rust_directory() {
    for dir in xtask::wrap::RUST_DIRS {
        let root = Root::rust(dir);
        assert!(!xtask::wrap::run(&root.0, 20).expect("reads"), "{dir}");
    }
    assert_eq!(xtask::wrap::RUST_DIRS, ["crates", "examples", "xtask"]);
}

/// A repository root holding the Markdown the mode reads, removed on drop.
struct Docs(std::path::PathBuf);

impl Docs {
    fn new(name: &str, index: &str, other: &str) -> Docs {
        let root = std::env::temp_dir().join(format!("seam-wrap-{name}-{}", std::process::id()));
        std::fs::create_dir_all(root.join("docs")).expect("a temporary directory");
        for file in xtask::wrap::MARKDOWN_FILES {
            std::fs::write(root.join(file), "short\n").expect("written");
        }
        std::fs::write(root.join("docs/decision-index.md"), index).expect("written");
        std::fs::write(root.join("docs/other.md"), other).expect("written");
        Docs(root)
    }
}

impl Drop for Docs {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn run_markdown_reads_the_docs_and_leaves_the_decision_indexs_site_lines_whole() {
    let site = "- `crates/host/src/state.rs`: `SavedState::save`, `SavedState::load`\n";
    let docs = Docs::new("index", site, "short\n");
    assert!(xtask::wrap::run_markdown(&docs.0, 30).expect("reads"));
    let docs = Docs::new("other", "short\n", site);
    assert!(!xtask::wrap::run_markdown(&docs.0, 30).expect("reads"));
}

#[test]
fn run_markdown_walks_the_repositorys_docs() {
    let root = xtask::root();
    // A width no line reaches passes; one of 40 fails for the many prose
    // lines whose words each fit it.
    assert!(xtask::wrap::run_markdown(&root, 100_000).expect("the docs read"));
    assert!(!xtask::wrap::run_markdown(&root, 40).expect("the docs read"));
}
