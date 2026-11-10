# Style

How Seam's code and docs are written. One direction comes before every rule
here: write idiomatic Rust, and shape the architecture the way Rust shapes it.
Seam's authors come from C++, so the review question for any design is whether
it is the Rust shape or a C++ shape written in Rust.

## Rust shapes, not C++ ones

A C++ instinct is often the wrong shape here. The left column is what to
notice; the right is what Rust offers instead, with an example from Seam.

| A C++ instinct | The Rust shape |
| --- | --- |
| Shared mutable state behind pointers, guarded by convention | One owner, borrows for access, and a handle per role: `ParamSet::take_ui` hands out the one UI side, and the type system keeps it on its thread. |
| Flags, sentinel values and pairs of bools | An enum whose variants carry their own data: the main thread's `Binding` (`crates/host/src/boundary.rs`) is bound to one thread or lent call by call, never both. |
| Error codes, `bool` returns and out-parameters | `Result` and `Option` with a typed error, propagated with `?`: `SavedState::load` returns `LoadError`. |
| Two-phase construction (`init` after the constructor) | A constructor or builder that returns a valid value or an error, so an invalid one cannot exist: `Range::new` refuses a bad range. |
| Raw integers for ids, units and bits | Newtypes that check once and carry the meaning: `ParamId`, `Position`, the packed value word. |
| Index loops and hand-checked bounds | Iterators, slices and `get`; an index only where the bound is proven, and the audio path's lints refuse the rest. |
| Base classes and virtual hierarchies | Traits and generics for open sets, enums for closed ones, composition over inheritance: `Plugin` and `Processor` are traits a plugin implements. |
| Cleanup by convention, paired begin and end calls | A guard or a closure that makes the pair one call: `Stager::block` (`crates/learn/src/capture.rs`) counts a block begun and ended around its offers. |
| A getter and setter for every field | Public fields on plain data; methods only where an invariant needs them. |

When a design needs `unsafe`, interior mutability or a lock, say why the safe,
owned shape does not fit. The [Rust API Guidelines][guidelines] and Clippy's
lints are the wider reference.

## Docs

Seam follows the API Guidelines' [documentation chapter][documentation] and the
standard library's conventions ([RFC 1574][rfc1574]):

- **Summary.** The first paragraph is one sentence in the third person
  ("Returns", "Writes"). The rest follows in its own paragraphs.
- **Sections.** `# Examples`, then `# Errors`, `# Panics` and `# Safety`
  whenever they apply. A contract line, such as the `Thread:` line, comes
  before `# Examples`.
- **Examples.** Every public type, trait, free function and macro has one, and
  so does a method whose summary does not tell the whole story. An example
  shows why you would use the item. Hidden lines may hide imports and setup,
  never a type whose behaviour an assertion depends on. Examples propagate
  errors with `?`.
- **Links.** An intra-doc link to every item a doc names. A crate's root
  re-exports with `#[doc(inline)]`, so its index shows each type's summary.
- **Crate overview.** What lives there, its invariants and one worked example.
  The prose fits a screen; the example may run longer.
- **Module docs.** A line or two, or a maintainer's overview where the module
  holds a state machine or an invariant, as `session/src/ui.rs` does.
- **Private items.** Usually nothing; a line where the name and type cannot
  carry the contract.
- **Comments.** Only a non-obvious why: an invariant, an ordering, a
  workaround or a bound.
- **`unsafe`.** `// SAFETY:` on every block and `# Safety` on every `unsafe
  fn`: which pointer is valid, for what and how long, on which thread, what
  aliasing is ruled out, and what the host or the header guarantees.

A plain getter or a constant gets a one-line summary. Plumbing public only so
Seam's other crates reach it is `#[doc(hidden)]`, and no public doc links to it.

## Seam's own rules

- **Reasons in words.** A comment or doc gives its reason in words. It cites no
  decision id, audit finding or stage; the
  [decision index](decision-index.md) runs the other way, from each decision
  to its code.
- **Once.** A rule is stated in one place, the item or doc that owns it, and
  linked from everywhere else. Restating it is a defect, since copies drift.
- **Open.** Something undecided or unverified stays visible, in a sentence that
  starts "Open:".
- **Threads.** A call that runs on one thread says which, in a `Thread:` line
  (`Thread: main.`, `Thread: any but audio.`).
- **Panics.** A documented panic must be able to happen, a `debug_assert!`
  included. Where Clippy's `missing_panics_doc` fires on one that cannot,
  `#[expect(..., reason = "...")]` says why.
- **Names.** `take_*` hands out what exists once, `None` while it lives. A
  getter has no `get_` prefix and no side effect. A key is the stable string a
  thing is declared under; an id is a cheap handle looked up from it once.
  `with_*` sets an option, `is_*` answers yes or no, and `tick` and `poll` are
  the once-per-UI-tick calls. A word the [glossary](glossary.md) defines keeps
  that sense; a new one goes in the glossary.
- **Width.** Prose wraps at 80 columns, in comments and Markdown alike. Tables,
  headings, code and a link that cannot break do not wrap.
- **Size.** Functions aim for 40 lines, 100 at most. A file splits past about
  500 code lines, or when it holds two ideas. Docs and tests do not count.
- **Features.** A feature-gated public item shows on docs.rs which feature it
  needs, through `#[cfg_attr(docsrs, doc(cfg(feature = "...")))]` under the
  crate root's `#![cfg_attr(docsrs, feature(doc_cfg))]`.

## The filter, for every comment

1. Does the code already say it? Delete it.
2. Could a name, a type or a test say it? Change the code, then delete it.
3. Is it history, a decision id or a status? It belongs in the commit message,
   the decision history or the record's table.
4. Is it a public contract? Rustdoc: summary, example, errors, panics, safety.
5. Is it a non-obvious why? Keep it, short.

## The repository's docs

| Doc | Holds |
| --- | --- |
| [README.md](../README.md) | What Seam is, its status, the layout, how to build, the licence; under 100 lines. |
| [CONTRIBUTING.md](../CONTRIBUTING.md) | How a change reaches `main`: the pull request, its title and the merge. |
| [AGENTS.md](../AGENTS.md) | Where a coding agent starts: it points at the rules and adds none. |
| [ARCHITECTURE.md](../ARCHITECTURE.md) | The bird's-eye view, the codemap, a host's call to a block, the invariants and the threads ([matklad's form][architecture]), naming files and items, never line numbers. |
| [first-plugin.md](first-plugin.md) | A walk through the example gain, in the order a plugin is written. |
| [testing.md](testing.md) | The test rules, and what each CI lane proves. |
| [glossary.md](glossary.md) | The words Seam coins, a sentence or two each, linking the item that owns it. |
| [The decision record](transfer-from-seam-juce.md) | What holds now: each decision's status, question, answer and alternatives. |
| [decision-history.md](decision-history.md) | How each decision was reached and changed; dates and review words belong here. |
| [decision-index.md](decision-index.md) | From each decision to its code and tests, one line per site. |
| [CHANGELOG.md](../CHANGELOG.md) | What each version holds, in [Keep a Changelog][changelog] form. |

## What checks it

- `missing_docs` and Clippy's `missing_errors_doc`, `missing_panics_doc`,
  `missing_safety_doc`, `undocumented_unsafe_blocks`, `doc_markdown` and
  `too_many_lines`, set in the workspace's `Cargo.toml` and errors in CI.
- `cargo doc` with `-D warnings`, so a broken intra-doc link fails, and
  `cargo test`, which runs every doc example.
- `scripts/check-example-coverage.py`: a public type, trait, free function,
  macro or alias with no example.
- `scripts/check-comment-tags.py`: a decision id, audit finding or stage in a
  comment or doc outside the record, its history and its index.
- `scripts/check-decision-index.py`: an index that disagrees with the record or
  names a site that is gone.
- `cargo xtask wrap --max 80` and `cargo xtask wrap --markdown --max 80`: prose
  past 80 columns.

The rest is review, and the first question there is the one at the top: is
this the Rust shape?

[guidelines]: https://rust-lang.github.io/api-guidelines/
[documentation]: https://rust-lang.github.io/api-guidelines/documentation.html
[rfc1574]: https://rust-lang.github.io/rfcs/1574-more-api-documentation-conventions.html
[architecture]: https://matklad.github.io/2021/02/06/ARCHITECTURE.md.html
[changelog]: https://keepachangelog.com/en/1.1.0/
