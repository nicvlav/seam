//! The Rust reader behind the check scripts: each case is a way a hand-written
//! tokenizer was fooled, or a behaviour the checks rely on.

use xtask::scan::{CommentKind, definitions, scan, scan_report};

fn comment_lines(source: &str) -> Vec<(usize, String)> {
    scan(source)
        .expect("the source lexes")
        .comments
        .into_iter()
        .map(|c| (c.line, c.text))
        .collect()
}

fn literals(source: &str) -> Vec<(usize, String)> {
    scan(source)
        .expect("the source lexes")
        .literals
        .into_iter()
        .map(|l| (l.line, l.text))
        .collect()
}

fn defines(source: &str, name: &str) -> bool {
    definitions(source)
        .expect("the source parses")
        .contains(name)
}

#[test]
fn a_quote_char_literal_does_not_hide_a_comment() {
    assert_eq!(
        comment_lines("let q = '\"'; // Q100 here\n"),
        [(1, "// Q100 here".to_string())]
    );
}

#[test]
fn a_string_with_an_escaped_backslash_ends() {
    assert_eq!(
        comment_lines("let p = \"\\\\\"; // after\n"),
        [(1, "// after".to_string())]
    );
}

#[test]
fn a_lifetime_opens_no_char_literal() {
    let source = "fn f<'a>(x: &'a str) -> &'a str { x } // tail\n";
    assert_eq!(comment_lines(source), [(1, "// tail".to_string())]);
}

#[test]
fn a_comment_after_a_string_over_two_lines_is_read() {
    assert_eq!(
        comment_lines("let s = \"abc\ndef\"; // Q100\n"),
        [(2, "// Q100".to_string())]
    );
}

#[test]
fn a_block_comment_is_read_line_by_line() {
    let found = comment_lines("/* one\n two Q100 */ let a = 1;\n");
    assert_eq!(
        found,
        [(1, "/* one".to_string()), (2, " two Q100 */".to_string())]
    );
}

#[test]
fn a_block_comment_nests() {
    let found = comment_lines("/* a /* b */ still a */ let x = 1; // c\n");
    assert_eq!(
        found,
        [
            (1, "/* a /* b */ still a */".to_string()),
            (1, "// c".to_string())
        ]
    );
}

#[test]
fn a_comment_marker_in_a_string_is_no_comment() {
    assert_eq!(
        comment_lines("let u = \"http://x\"; let v = \"/* y\";\n"),
        []
    );
}

#[test]
fn doc_comments_are_comments_of_their_own_kind() {
    let found = scan("//! inner\n/// outer\n//// plain\nfn f() {}\n").expect("lexes");
    let kinds: Vec<_> = found.comments.iter().map(|c| (c.line, c.kind)).collect();
    assert_eq!(
        kinds,
        [
            (1, CommentKind::InnerDoc),
            (2, CommentKind::OuterDoc),
            (3, CommentKind::Line)
        ]
    );
}

#[test]
fn an_explicit_doc_attribute_is_read_whole_by_line() {
    let found = scan("#[doc = \"cites Q100\"]\nfn f() {}\n").expect("lexes");
    assert_eq!(found.comments.len(), 1);
    assert_eq!(found.comments[0].kind, CommentKind::DocAttribute);
    assert_eq!(found.comments[0].text, "#[doc = \"cites Q100\"]");
}

#[test]
fn string_literals_and_names_are_read() {
    let source = "fn each_origin_falls_back_as_q99_lists() {\n    assert!(x, \"snapped (Q100)\");\n    let k = \"Q99\";\n}\n";
    let found = scan(source).expect("lexes");
    let names: Vec<_> = found
        .names
        .iter()
        .map(|n| (n.line, n.text.as_str()))
        .collect();
    assert_eq!(names, [(1, "each_origin_falls_back_as_q99_lists")]);
    assert_eq!(
        literals(source),
        [(2, "snapped (Q100)".to_string()), (3, "Q99".to_string())]
    );
}

#[test]
fn a_literal_in_a_comment_is_not_read() {
    assert_eq!(literals("// \"Q100\"\nlet a = 1;\n"), []);
    assert_eq!(literals("/// \"Q100\"\nfn f() {}\n"), []);
}

#[test]
fn a_raw_string_is_read_whole() {
    assert_eq!(
        literals("let a = r#\"x \"quoted\" (Q100)\"#;\n"),
        [(1, "x \"quoted\" (Q100)".to_string())]
    );
}

#[test]
fn a_byte_or_c_raw_string_is_read_whole() {
    for prefix in ["br", "cr"] {
        let source = format!("let t = {prefix}#\"say \"hi\"#; // Q6\n");
        assert_eq!(
            comment_lines(&source),
            [(1, "// Q6".to_string())],
            "{prefix}"
        );
        assert_eq!(literals(&source), [(1, "say \"hi".to_string())], "{prefix}");
    }
}

#[test]
fn an_identifier_ending_in_r_does_not_open_a_raw_string() {
    assert_eq!(
        comment_lines("let a = bar(\"x\"); // tail\n"),
        [(1, "// tail".to_string())]
    );
}

#[test]
fn a_literal_inside_a_macro_call_is_read() {
    assert_eq!(
        literals("fn f() { assert!(x, \"snapped (Q100)\"); }\n"),
        [(1, "snapped (Q100)".to_string())]
    );
}

#[test]
fn source_that_does_not_lex_is_an_error() {
    assert!(scan("let s = \"never closed;\n").is_err());
    assert!(scan("/* never closed\n").is_err());
}

#[test]
fn the_report_escapes_what_would_break_a_line() {
    let report = scan_report("a.rs", "/* x\\y\n z */\n").expect("lexes");
    assert_eq!(
        report,
        "file\ta.rs\ncomment\t1\t/* x\\\\y\ncomment\t2\t z */\n"
    );
}

#[test]
fn a_use_list_does_not_define_a_field_or_variant() {
    let source = "use seam::{\n    Commit, ParamId,\n};\nfn f() {}\n";
    assert!(!defines(source, "ParamId"));
    assert!(!defines(source, "Commit"));
}

#[test]
fn a_struct_literal_does_not_define_a_field() {
    let source = "struct A { x: u8 }\nfn f() -> A {\n    A {\n        gain: 1,\n    }\n}\n";
    assert!(!defines(source, "gain"));
    assert!(defines(source, "x"));
}

#[test]
fn fields_and_variants_are_found_in_their_bodies() {
    let source = "pub struct A {\n    pub gain: f32,\n}\nenum E {\n    Wide,\n    Narrow(u8),\n    Named { depth: u8 },\n}\n";
    for name in ["gain", "Wide", "Narrow", "Named", "depth"] {
        assert!(defines(source, name), "{name}");
    }
}

#[test]
fn items_of_every_kind_are_found() {
    let source = "pub(crate) fn tick() {}\nstruct S;\nenum E {}\ntrait T { fn m(); type Out; const C: u8; }\ntype Al = u8;\nconst K: u8 = 1;\nstatic G: u8 = 1;\nunion U { a: u8 }\nmod inner {}\nmacro_rules! mk { () => {} }\nimpl T for S { fn m() {} type Out = u8; const C: u8 = 1; }\nextern \"C\" { fn ext(); }\n";
    for name in [
        "tick", "S", "E", "T", "m", "Out", "C", "Al", "K", "G", "U", "a", "inner", "mk", "ext",
    ] {
        assert!(defines(source, name), "{name}");
    }
}

#[test]
fn a_function_in_a_body_is_found() {
    assert!(defines("fn outer() { fn inner() {} }\n", "inner"));
}

#[test]
fn an_impl_header_names_its_type_and_trait() {
    let source = "impl<T: Clone> Wrapper<T> for Thing {}\nimpl Plain {}\n";
    for name in ["Wrapper", "Thing", "Plain"] {
        assert!(defines(source, name), "{name}");
    }
}

#[test]
fn a_keyword_and_name_in_a_comment_define_nothing() {
    let source = "// the fn tick runs here\n/// see `struct Gain`\nfn other() {}\n";
    assert!(!defines(source, "tick"));
    assert!(!defines(source, "Gain"));
}

#[test]
fn a_keyword_and_name_in_a_string_define_nothing() {
    let source = "const MESSAGE: &str = \"fn tick() { struct Gain { depth: u8 } }\";\n";
    for name in ["tick", "Gain", "depth"] {
        assert!(!defines(source, name), "{name}");
    }
}

#[test]
fn a_brace_in_a_comment_or_string_does_not_stretch_a_body() {
    let source = "struct A {\n    /// opens a { brace\n    x: u8,\n}\nconst T: &str = \"{\";\nfn f() -> B {\n    B {\n        y: 1,\n    }\n}\n";
    assert!(defines(source, "x"));
    assert!(!defines(source, "y"));
}

#[test]
fn items_a_macro_call_spells_are_found() {
    let source = "c_structs! {\n    /// A param.\n    pub struct Info {\n        /// Key.\n        pub key: u32,\n        pub kind: u32,\n    }\n    pub enum Mode { Fast, Slow }\n}\nmacro_rules! make { () => { fn made() {} } }\n";
    for name in [
        "Info", "key", "kind", "Mode", "Fast", "Slow", "made", "make",
    ] {
        assert!(defines(source, name), "{name}");
    }
}

#[test]
fn source_that_is_not_a_file_is_an_error() {
    assert!(definitions("fn (\n").is_err());
}

#[test]
fn the_repository_reads_without_error() {
    // The whole tree, so a construct the reader cannot lex fails here and not
    // only in the scripts' lane.
    let root = xtask::root();
    let files = xtask::wrap::rust_files(&root.join("crates")).expect("crates/ walks");
    assert!(!files.is_empty(), "no file found, so nothing was read");
    for path in files {
        let source = std::fs::read_to_string(&path).expect("a source file reads");
        scan(&source).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        // The doc fragments under `doc/` hold doc-test lines (`# use ...`),
        // which are not Rust files; only the lexer reads them.
        if !path.components().any(|c| c.as_os_str() == "doc") {
            definitions(&source).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        }
    }
}

#[test]
fn a_byte_order_mark_is_neither_a_token_nor_a_comment() {
    assert_eq!(
        comment_lines("\u{feff}// first\nfn f() {}\n"),
        [(1, "// first".to_string())]
    );
}
