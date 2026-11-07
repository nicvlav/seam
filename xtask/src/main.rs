//! `cargo xtask <task>`: see the library's documentation for the tasks.

use std::io::Read;
use std::process::ExitCode;

const USAGE: &str = "usage: cargo xtask <task>

  gen               rewrite every generated file
  scan              read Rust file paths from stdin, print their comments, string literals and fn names
  defs              read Rust file paths from stdin, print the names each defines
  wrap [--max N]    list the comment lines of crates/, examples/ and xtask/ longer than N columns (default 80)
  wrap --markdown [--max N]
                    list the prose lines of README.md, ARCHITECTURE.md, CHANGELOG.md and docs/*.md longer than N columns";

fn main() -> ExitCode {
    let mut args = std::env::args().skip(1);
    let result = match args.next().as_deref() {
        #[cfg(feature = "generate")]
        Some("gen") => generate(),
        Some("scan") => per_file(xtask::scan::scan_report),
        Some("defs") => per_file(xtask::scan::definitions_report),
        Some("wrap") => wrap(args.collect()),
        _ => {
            eprintln!("{USAGE}");
            return ExitCode::from(2);
        }
    };
    match result {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => ExitCode::FAILURE,
        Err(message) => {
            eprintln!("{message}");
            ExitCode::from(2)
        }
    }
}

#[cfg(feature = "generate")]
fn generate() -> Result<bool, String> {
    let root = xtask::root();
    for file in xtask::generated() {
        let path = root.join(file.path);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
        }
        std::fs::write(&path, file.contents).map_err(|e| format!("{}: {e}", path.display()))?;
        println!("wrote {}", file.path);
    }
    Ok(true)
}

/// Reads one path per line from stdin and prints `report` for each file.
/// An empty list is an error: a check handed nothing would pass.
fn per_file(report: fn(&str, &str) -> Result<String, String>) -> Result<bool, String> {
    let mut input = String::new();
    std::io::stdin()
        .read_to_string(&mut input)
        .map_err(|e| format!("stdin: {e}"))?;
    let paths: Vec<&str> = input.lines().filter(|l| !l.is_empty()).collect();
    if paths.is_empty() {
        return Err("no file path on stdin".into());
    }
    let mut out = String::new();
    for path in paths {
        let source = std::fs::read_to_string(path).map_err(|e| format!("{path}: {e}"))?;
        out.push_str(&report(path, &source)?);
    }
    print!("{out}");
    Ok(true)
}

fn wrap(args: Vec<String>) -> Result<bool, String> {
    let mut max = xtask::wrap::DEFAULT_MAX;
    let mut markdown = false;
    let mut args = args.into_iter();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--markdown" => markdown = true,
            "--max" => {
                let value = args.next().ok_or(USAGE)?;
                max = value
                    .parse()
                    .map_err(|_| format!("--max wants a number, not {value:?}"))?;
            }
            _ => return Err(USAGE.into()),
        }
    }
    if markdown {
        xtask::wrap::run_markdown(&xtask::root(), max)
    } else {
        xtask::wrap::run(&xtask::root(), max)
    }
}
