//! Seam's repository tasks, run as `cargo xtask <task>`.
//!
//! `gen` rewrites every file the repository commits that Rust generates:
//! the page's protocol types and its test parameter map, the C header the
//! JUCE adapter builds against, and the example EQ's JUCE build settings.
//! Each has a drift gate (`tests/drift.rs`), so a stale one fails CI.
//!
//! Three tasks read Rust source exactly, with `syn` and `proc-macro2`, for the
//! repository's check scripts and for the style gate:
//!
//! - `scan` lists each file's comments, string literals and `fn` names;
//! - `defs` lists the names each file defines;
//! - `wrap` lists the comment lines of `crates/`, `examples/` and `xtask/`
//!   that run past the wrap width, and with `--markdown` the prose lines of
//!   the repository's Markdown.
//!
//! They need no other crate of the workspace, so a lane that only reads source
//! builds xtask with `--no-default-features` and skips the generators.

use std::path::{Path, PathBuf};

#[cfg(feature = "generate")]
mod generate;
#[cfg(feature = "generate")]
pub use generate::{Generated, generated, page_test_params};

pub mod scan;
pub mod wrap;

/// Returns the repository root.
#[expect(
    clippy::missing_panics_doc,
    reason = "the manifest directory is `xtask/` inside the repository, so it has a parent"
)]
pub fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("xtask sits in the repository root")
        .to_path_buf()
}
