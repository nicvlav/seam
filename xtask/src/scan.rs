//! Reading Rust source exactly, for the check scripts.
//!
//! `proc-macro2` lexes the file, so a string that runs over lines, a raw
//! string with any prefix, a char literal, a lifetime and a nested block
//! comment are each read by the language's own rules. Its lexer drops comments
//! but gives every token its byte range, and everything between two tokens
//! is white space or a comment, so the comments are the source between the
//! tokens. A doc comment is a token; its source text says it is one.
//! `syn` parses the items for [`definitions`].
//!
//! The two tasks print one fact per line, tab separated, after a
//! `file<TAB>path` line for each file read. Text is escaped: `\\`, `\n`, `\r`
//! and `\t`.

use std::collections::BTreeSet;
use std::fmt::Write as _;
use std::ops::Range;

use proc_macro2::{Delimiter, TokenStream, TokenTree};
use quote::ToTokens;
use syn::visit::Visit;

/// What a comment is, by how it is written.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CommentKind {
    /// `//`, and `////` or more, which Rust does not read as a doc.
    Line,
    /// `///`.
    OuterDoc,
    /// `//!`.
    InnerDoc,
    /// `/* */`, a doc block included. One entry per source line.
    Block,
    /// A line holding a `#[doc ...]` attribute, whole. One entry per source
    /// line.
    DocAttribute,
}

/// One line of a comment.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Comment {
    /// The 1-based source line.
    pub line: usize,
    /// How the comment is written.
    pub kind: CommentKind,
    /// The line's text of the comment, delimiters included. For a
    /// [`CommentKind::DocAttribute`] it is the whole source line.
    pub text: String,
}

/// A string literal's value, or a `fn` name, with the line it starts on.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Item {
    /// The 1-based source line.
    pub line: usize,
    /// The unescaped value of a string literal, or a `fn`'s name.
    pub text: String,
}

/// What a file holds that the checks read.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Scan {
    /// Every comment, in source order, one entry per line.
    pub comments: Vec<Comment>,
    /// Every string literal (also byte and C strings, raw or not).
    pub literals: Vec<Item>,
    /// The name after every `fn`.
    pub names: Vec<Item>,
}

/// Reads the comments, string literals and `fn` names of Rust source.
///
/// # Errors
///
/// Returns why the source does not lex (an unterminated string or comment),
/// which a check must not read past.
pub fn scan(source: &str) -> Result<Scan, String> {
    let stream: TokenStream = source.parse().map_err(|e| format!("{e}"))?;
    let lines = Lines::new(source);
    let mut walk = Walk {
        source,
        lines: &lines,
        leaves: Vec::new(),
        docs: BTreeSet::new(),
        out: Scan::default(),
    };
    walk.stream(stream);
    walk.finish()
}

/// Where each line of a source starts.
struct Lines(Vec<usize>);

impl Lines {
    fn new(source: &str) -> Self {
        let breaks = source.match_indices('\n').map(|(at, _)| at + 1);
        Self(std::iter::once(0).chain(breaks).collect())
    }

    /// The 1-based line holding byte `at`.
    fn of(&self, at: usize) -> usize {
        self.0.partition_point(|&start| start <= at)
    }

    /// The byte range of 1-based line `number`, without its line break.
    fn range(&self, number: usize, source: &str) -> Range<usize> {
        let start = self.0[number - 1];
        let end = self.0.get(number).map_or(source.len(), |&next| next - 1);
        start..end
    }
}

struct Walk<'a> {
    source: &'a str,
    lines: &'a Lines,
    /// The byte range of every token (a group's two delimiters, not its body).
    leaves: Vec<Range<usize>>,
    /// The ranges of the doc comments, which the lexer turns into tokens.
    docs: BTreeSet<(usize, usize)>,
    out: Scan,
}

impl Walk<'_> {
    fn stream(&mut self, stream: TokenStream) {
        let trees: Vec<TokenTree> = stream.into_iter().collect();
        for (at, tree) in trees.iter().enumerate() {
            match tree {
                TokenTree::Group(group) => {
                    self.leaf(&group.span_open().byte_range());
                    self.leaf(&group.span_close().byte_range());
                    self.stream(group.stream());
                }
                TokenTree::Ident(ident) => {
                    self.leaf(&ident.span().byte_range());
                    if ident == "fn"
                        && let Some(TokenTree::Ident(name)) = trees.get(at + 1)
                    {
                        self.out.names.push(Item {
                            line: self.lines.of(ident.span().byte_range().start),
                            text: name.to_string().trim_start_matches("r#").to_string(),
                        });
                    }
                }
                TokenTree::Punct(punct) => {
                    let range = punct.span().byte_range();
                    if !self.leaf(&range) && punct.as_char() == '#' {
                        self.attribute(&trees[at + 1..], range.start);
                    }
                }
                TokenTree::Literal(literal) => {
                    let range = literal.span().byte_range();
                    if !self.leaf(&range)
                        && let Some(text) = string_value(literal)
                    {
                        self.out.literals.push(Item {
                            line: self.lines.of(range.start),
                            text,
                        });
                    }
                }
            }
        }
    }

    /// Records a token's range. Returns whether the token is part of a doc
    /// comment, which the lexer writes as `#[doc = "..."]` over the comment's
    /// own range.
    fn leaf(&mut self, range: &Range<usize>) -> bool {
        self.leaves.push(range.clone());
        let text = &self.source[range.clone()];
        let is_doc = text.starts_with("//") || text.starts_with("/*");
        if is_doc {
            self.docs.insert((range.start, range.end));
        }
        is_doc
    }

    /// An explicit `#[doc ...]` or `#![doc ...]` after the `#` at `start`:
    /// each of its lines is read whole, as a comment.
    fn attribute(&mut self, after: &[TokenTree], start: usize) {
        let mut rest = after.iter();
        let mut next = rest.next();
        if matches!(next, Some(TokenTree::Punct(p)) if p.as_char() == '!') {
            next = rest.next();
        }
        let Some(TokenTree::Group(group)) = next else {
            return;
        };
        if group.delimiter() != Delimiter::Bracket
            || !matches!(group.stream().into_iter().next(), Some(TokenTree::Ident(i)) if i == "doc")
        {
            return;
        }
        let last = group.span_close().byte_range().start;
        for number in self.lines.of(start)..=self.lines.of(last) {
            let range = self.lines.range(number, self.source);
            self.out.comments.push(Comment {
                line: number,
                kind: CommentKind::DocAttribute,
                text: self.source[range].trim_end_matches('\r').to_string(),
            });
        }
    }

    /// Reads the comments from the doc comments and the gaps between tokens.
    fn finish(mut self) -> Result<Scan, String> {
        // A byte order mark is not a token and not a comment.
        let mut cursor = if self.source.starts_with('\u{feff}') {
            '\u{feff}'.len_utf8()
        } else {
            0
        };
        let mut leaves = std::mem::take(&mut self.leaves);
        leaves.sort_by_key(|range| range.start);
        for leaf in &leaves {
            if leaf.start > cursor {
                self.gap(cursor..leaf.start)?;
            }
            cursor = cursor.max(leaf.end);
        }
        self.gap(cursor..self.source.len())?;
        for (start, end) in std::mem::take(&mut self.docs) {
            self.comment(start..end);
        }
        self.out
            .comments
            .sort_by_key(|comment| (comment.line, comment.kind == CommentKind::DocAttribute));
        Ok(self.out)
    }

    /// The comments in a stretch of source that holds no token.
    fn gap(&mut self, range: Range<usize>) -> Result<(), String> {
        let source = self.source;
        let mut at = range.start;
        while at < range.end {
            let rest = &source[at..range.end];
            let skipped = rest.len() - rest.trim_start().len();
            at += skipped;
            let rest = &source[at..range.end];
            if rest.is_empty() {
                break;
            }
            let length = if rest.starts_with("//") {
                rest.find('\n').unwrap_or(rest.len())
            } else if rest.starts_with("/*") {
                block_length(rest).ok_or("a block comment is not closed")?
            } else {
                return Err(format!(
                    "line {}: unexpected text between tokens: {:?}",
                    self.lines.of(at),
                    &rest[..rest.len().min(20)]
                ));
            };
            self.comment(at..at + length);
            at += length;
        }
        Ok(())
    }

    fn comment(&mut self, range: Range<usize>) {
        let text = &self.source[range.clone()];
        let kind = if text.starts_with("/*") {
            CommentKind::Block
        } else if text.starts_with("///") && !text.starts_with("////") {
            CommentKind::OuterDoc
        } else if text.starts_with("//!") {
            CommentKind::InnerDoc
        } else {
            CommentKind::Line
        };
        let first = self.lines.of(range.start);
        for (offset, part) in text.split('\n').enumerate() {
            self.out.comments.push(Comment {
                line: first + offset,
                kind,
                text: part.trim_end_matches('\r').to_string(),
            });
        }
    }
}

/// The length of the block comment `text` starts with, nesting allowed.
fn block_length(text: &str) -> Option<usize> {
    let bytes = text.as_bytes();
    let (mut depth, mut at) = (0_usize, 0_usize);
    while at + 1 < bytes.len() {
        match &bytes[at..at + 2] {
            b"/*" => {
                depth += 1;
                at += 2;
            }
            b"*/" => {
                depth -= 1;
                at += 2;
                if depth == 0 {
                    return Some(at);
                }
            }
            _ => at += 1,
        }
    }
    None
}

/// A string, byte string or C string literal's value.
fn string_value(literal: &proc_macro2::Literal) -> Option<String> {
    match syn::Lit::new(literal.clone()) {
        syn::Lit::Str(s) => Some(s.value()),
        syn::Lit::ByteStr(b) => Some(String::from_utf8_lossy(&b.value()).into_owned()),
        syn::Lit::CStr(c) => Some(c.value().to_string_lossy().into_owned()),
        _ => None,
    }
}

/// Returns the names `source` defines: items of every kind (functions,
/// types, traits, constants, statics, modules, macros, associated items,
/// foreign items), the fields of structs, unions and enum variants, the
/// variants themselves, and every name in an `impl` header.
///
/// Items an item-position or statement macro's tokens spell
/// (`fn name`, `struct Name`, and the like, anywhere in the call) count too, so
/// a `macro_rules!` body that defines a function does. A word in a comment or a
/// string defines nothing.
///
/// # Errors
///
/// Returns why the source is not a Rust file.
pub fn definitions(source: &str) -> Result<BTreeSet<String>, String> {
    let file =
        syn::parse_file(source).map_err(|e| format!("line {}: {e}", e.span().start().line))?;
    let mut names = Definitions::default();
    names.visit_file(&file);
    Ok(names.0)
}

#[derive(Default)]
struct Definitions(BTreeSet<String>);

impl Definitions {
    fn add(&mut self, ident: &syn::Ident) {
        self.0
            .insert(ident.to_string().trim_start_matches("r#").to_string());
    }

    fn idents(&mut self, tokens: TokenStream) {
        for tree in tokens {
            match tree {
                TokenTree::Ident(ident) => self.add(&ident),
                TokenTree::Group(group) => self.idents(group.stream()),
                _ => {}
            }
        }
    }

    fn fields(&mut self, fields: &syn::Fields) {
        if let syn::Fields::Named(named) = fields {
            for field in named.named.iter().filter_map(|f| f.ident.as_ref()) {
                self.add(field);
            }
        }
    }

    /// What a macro call's tokens define: a definer word and the name after it.
    fn macro_tokens(&mut self, tokens: TokenStream) {
        let trees: Vec<TokenTree> = tokens.into_iter().collect();
        for (at, tree) in trees.iter().enumerate() {
            match tree {
                TokenTree::Group(group) => self.macro_tokens(group.stream()),
                TokenTree::Ident(word) if DEFINERS.iter().any(|d| word == d) => {
                    if let Some(TokenTree::Ident(name)) = trees.get(at + 1) {
                        self.add(name);
                        self.body_names(word == "enum", &trees[at + 2..]);
                    }
                }
                _ => {}
            }
        }
    }

    /// The field names, or for an enum the variant names, of the braces that
    /// follow a `struct`, `union` or `enum` a macro call spells, before any
    /// `;` or other item.
    fn body_names(&mut self, variants: bool, after: &[TokenTree]) {
        let Some(group) = after
            .iter()
            .take_while(|t| !matches!(t, TokenTree::Punct(p) if p.as_char() == ';'))
            .find_map(|t| match t {
                TokenTree::Group(g) if g.delimiter() == Delimiter::Brace => Some(g),
                _ => None,
            })
        else {
            return;
        };
        let entries: Vec<TokenTree> = group.stream().into_iter().collect();
        // A field is `name :` and a variant starts an entry, which a `,`
        // ends; an attribute (`#` and its brackets) and `pub` come before.
        let mut starts_entry = true;
        for (at, tree) in entries.iter().enumerate() {
            match tree {
                TokenTree::Punct(p) if p.as_char() == ',' => starts_entry = true,
                TokenTree::Ident(name) if variants && starts_entry && name != "pub" => {
                    self.add(name);
                    starts_entry = false;
                }
                TokenTree::Ident(name) if !variants => {
                    let colon = matches!(
                        entries.get(at + 1),
                        Some(TokenTree::Punct(p))
                            if p.as_char() == ':' && p.spacing() == proc_macro2::Spacing::Alone
                    );
                    if colon {
                        self.add(name);
                    }
                }
                _ => {}
            }
        }
    }
}

/// The words after which a macro's tokens name something they define.
const DEFINERS: [&str; 9] = [
    "fn", "struct", "enum", "trait", "type", "const", "static", "union", "mod",
];

impl<'ast> Visit<'ast> for Definitions {
    fn visit_item_fn(&mut self, node: &'ast syn::ItemFn) {
        self.add(&node.sig.ident);
        syn::visit::visit_item_fn(self, node);
    }
    fn visit_impl_item_fn(&mut self, node: &'ast syn::ImplItemFn) {
        self.add(&node.sig.ident);
        syn::visit::visit_impl_item_fn(self, node);
    }
    fn visit_trait_item_fn(&mut self, node: &'ast syn::TraitItemFn) {
        self.add(&node.sig.ident);
        syn::visit::visit_trait_item_fn(self, node);
    }
    fn visit_foreign_item_fn(&mut self, node: &'ast syn::ForeignItemFn) {
        self.add(&node.sig.ident);
        syn::visit::visit_foreign_item_fn(self, node);
    }
    fn visit_item_struct(&mut self, node: &'ast syn::ItemStruct) {
        self.add(&node.ident);
        self.fields(&node.fields);
        syn::visit::visit_item_struct(self, node);
    }
    fn visit_item_union(&mut self, node: &'ast syn::ItemUnion) {
        self.add(&node.ident);
        for field in node.fields.named.iter().filter_map(|f| f.ident.as_ref()) {
            self.add(field);
        }
        syn::visit::visit_item_union(self, node);
    }
    fn visit_item_enum(&mut self, node: &'ast syn::ItemEnum) {
        self.add(&node.ident);
        for variant in &node.variants {
            self.add(&variant.ident);
            self.fields(&variant.fields);
        }
        syn::visit::visit_item_enum(self, node);
    }
    fn visit_item_trait(&mut self, node: &'ast syn::ItemTrait) {
        self.add(&node.ident);
        syn::visit::visit_item_trait(self, node);
    }
    fn visit_item_trait_alias(&mut self, node: &'ast syn::ItemTraitAlias) {
        self.add(&node.ident);
        syn::visit::visit_item_trait_alias(self, node);
    }
    fn visit_item_type(&mut self, node: &'ast syn::ItemType) {
        self.add(&node.ident);
        syn::visit::visit_item_type(self, node);
    }
    fn visit_impl_item_type(&mut self, node: &'ast syn::ImplItemType) {
        self.add(&node.ident);
        syn::visit::visit_impl_item_type(self, node);
    }
    fn visit_trait_item_type(&mut self, node: &'ast syn::TraitItemType) {
        self.add(&node.ident);
        syn::visit::visit_trait_item_type(self, node);
    }
    fn visit_foreign_item_type(&mut self, node: &'ast syn::ForeignItemType) {
        self.add(&node.ident);
        syn::visit::visit_foreign_item_type(self, node);
    }
    fn visit_item_const(&mut self, node: &'ast syn::ItemConst) {
        self.add(&node.ident);
        syn::visit::visit_item_const(self, node);
    }
    fn visit_impl_item_const(&mut self, node: &'ast syn::ImplItemConst) {
        self.add(&node.ident);
        syn::visit::visit_impl_item_const(self, node);
    }
    fn visit_trait_item_const(&mut self, node: &'ast syn::TraitItemConst) {
        self.add(&node.ident);
        syn::visit::visit_trait_item_const(self, node);
    }
    fn visit_item_static(&mut self, node: &'ast syn::ItemStatic) {
        self.add(&node.ident);
        syn::visit::visit_item_static(self, node);
    }
    fn visit_foreign_item_static(&mut self, node: &'ast syn::ForeignItemStatic) {
        self.add(&node.ident);
        syn::visit::visit_foreign_item_static(self, node);
    }
    fn visit_item_mod(&mut self, node: &'ast syn::ItemMod) {
        self.add(&node.ident);
        syn::visit::visit_item_mod(self, node);
    }
    fn visit_item_macro(&mut self, node: &'ast syn::ItemMacro) {
        if let Some(ident) = &node.ident {
            self.add(ident);
        }
        syn::visit::visit_item_macro(self, node);
    }
    fn visit_item_impl(&mut self, node: &'ast syn::ItemImpl) {
        self.idents(node.generics.to_token_stream());
        if let Some((_, path, _)) = &node.trait_ {
            self.idents(path.to_token_stream());
        }
        self.idents(node.self_ty.to_token_stream());
        syn::visit::visit_item_impl(self, node);
    }
    fn visit_macro(&mut self, node: &'ast syn::Macro) {
        self.macro_tokens(node.tokens.clone());
        syn::visit::visit_macro(self, node);
    }
}

/// Escapes text for one output field.
#[must_use]
pub fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for ch in text.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            other => out.push(other),
        }
    }
    out
}

/// Prints `scan`'s facts for `path`'s `source` in the task's line format.
///
/// # Errors
///
/// As [`scan()`].
pub fn scan_report(path: &str, source: &str) -> Result<String, String> {
    let scan = scan(source).map_err(|e| format!("{path}: {e}"))?;
    let mut out = format!("file\t{path}\n");
    for comment in &scan.comments {
        let _ = writeln!(out, "comment\t{}\t{}", comment.line, escape(&comment.text));
    }
    for literal in &scan.literals {
        let _ = writeln!(out, "literal\t{}\t{}", literal.line, escape(&literal.text));
    }
    for name in &scan.names {
        let _ = writeln!(out, "name\t{}\t{}", name.line, escape(&name.text));
    }
    Ok(out)
}

/// Prints `defs`' names for `path`'s `source` in the task's line format.
///
/// # Errors
///
/// As [`definitions`].
pub fn definitions_report(path: &str, source: &str) -> Result<String, String> {
    let names = definitions(source).map_err(|e| format!("{path}: {e}"))?;
    let mut out = format!("file\t{path}\n");
    for name in names {
        let _ = writeln!(out, "def\t{name}");
    }
    Ok(out)
}
