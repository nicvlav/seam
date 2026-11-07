//! The drift gate: a generated file committed to the repository matches what
//! the code generates now.
//!
//! Generated files are committed so a reader, a reviewer and the page's own
//! build see them without running Rust. A gate fails when a Rust change alters
//! the output and the committed file was not regenerated. Regenerate by
//! running the failing test with [`BLESS_VAR`] set, then review the diff and
//! commit it.

use std::ffi::OsStr;
use std::path::Path;

/// The environment variable that rewrites committed files instead of checking
/// them.
///
/// Empty or `0` counts as unset, so `SEAM_BLESS=0` checks as a plain run does.
pub const BLESS_VAR: &str = "SEAM_BLESS";

/// The environment variable CI services set.
///
/// While it is set and not empty, a rewrite is refused: a gate that rewrites
/// its own file passes whatever the code generates.
pub const CI_VAR: &str = "CI";

/// Checks that the file at `path` holds exactly `generated`, or rewrites it
/// when [`BLESS_VAR`] asks for it.
///
/// Line endings are compared as LF.
///
/// # Panics
///
/// If the file is missing or differs, with the command that regenerates it;
/// if [`BLESS_VAR`] asks for a rewrite while [`CI_VAR`] is set; or if a
/// rewrite cannot write the file.
///
/// # Examples
///
/// ```
/// use seam_testkit::drift::check_generated;
///
/// # fn main() -> std::io::Result<()> {
/// // The committed file, as a checkout on Windows may hold it.
/// let path = std::env::temp_dir().join("seam-testkit-drift-example.txt");
/// std::fs::write(&path, "export const VERSION = 1;\r\n")?;
///
/// // Line endings compare as LF; a real gate generates the text from the code.
/// check_generated(&path, "export const VERSION = 1;\n");
/// # std::fs::remove_file(&path)?;
/// # Ok(())
/// # }
/// ```
pub fn check_generated(path: &Path, generated: &str) {
    let mode = mode(
        std::env::var_os(BLESS_VAR).as_deref(),
        std::env::var_os(CI_VAR).as_deref(),
    );
    match mode.unwrap_or_else(|refusal| panic!("{refusal}")) {
        Mode::Bless => {
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent).expect("create the generated file's folder");
            }
            std::fs::write(path, generated).expect("write the generated file");
        }
        Mode::Check => {
            let committed = std::fs::read_to_string(path)
                .unwrap_or_else(|error| {
                    panic!("{} cannot be read ({error}); {}", path.display(), hint())
                })
                .replace("\r\n", "\n");
            assert!(
                committed == generated,
                "{} is out of date with the Rust declarations (first difference at line {}); {}",
                path.display(),
                first_difference(&committed, generated),
                hint()
            );
        }
    }
}

/// What a gate does with its committed file.
#[derive(Debug, PartialEq, Eq)]
enum Mode {
    Check,
    Bless,
}

/// The mode the values of [`BLESS_VAR`] and [`CI_VAR`] ask for, or why it is
/// refused.
fn mode(bless: Option<&OsStr>, ci: Option<&OsStr>) -> Result<Mode, String> {
    let is_set = |value: Option<&OsStr>| value.is_some_and(|value| !value.is_empty());
    let blesses = is_set(bless) && bless != Some(OsStr::new("0"));
    match (blesses, is_set(ci)) {
        (false, _) => Ok(Mode::Check),
        (true, false) => Ok(Mode::Bless),
        (true, true) => Err(format!(
            "{BLESS_VAR} is set while {CI_VAR} is: a gate that rewrites its own file \
             passes whatever the code generates, so CI refuses it"
        )),
    }
}

fn hint() -> String {
    format!("rerun this test with {BLESS_VAR}=1 to regenerate it, then review and commit the diff")
}

fn first_difference(a: &str, b: &str) -> usize {
    a.lines()
        .zip(b.lines())
        .position(|(x, y)| x != y)
        .unwrap_or_else(|| a.lines().count().min(b.lines().count()))
        + 1
}

#[cfg(test)]
mod tests {
    use super::*;

    fn os(value: &str) -> Option<&OsStr> {
        Some(OsStr::new(value))
    }

    #[test]
    fn empty_or_zero_does_not_bless() {
        for bless in [None, os(""), os("0")] {
            assert_eq!(mode(bless, None), Ok(Mode::Check), "{bless:?}");
        }
        for bless in [os("1"), os("yes")] {
            assert_eq!(mode(bless, None), Ok(Mode::Bless), "{bless:?}");
        }
    }

    #[test]
    fn a_rewrite_is_refused_in_ci_and_only_there() {
        // Any value of CI but an empty one is a CI run.
        for ci in [os("true"), os("1")] {
            assert!(mode(os("1"), ci).is_err(), "{ci:?}");
        }
        for ci in [None, os("")] {
            assert_eq!(mode(os("1"), ci), Ok(Mode::Bless), "{ci:?}");
        }
        for bless in [None, os("0")] {
            assert_eq!(mode(bless, os("true")), Ok(Mode::Check), "{bless:?}");
        }
    }

    #[test]
    fn the_refusal_reads_as_one_sentence() {
        let refusal = mode(os("1"), os("true")).unwrap_err();
        assert!(!refusal.contains("  "), "a run of spaces in {refusal:?}");
    }

    #[test]
    fn names_the_first_line_that_differs() {
        assert_eq!(first_difference("a\nb\nc", "a\nx\nc"), 2);
        assert_eq!(first_difference("a\nb", "a\nb\nc"), 3);
    }
}
