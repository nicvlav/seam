//! The Rust side of the bench's parity claim, on every change: the `parity`
//! example's output (`examples/parity/`), which `bench/scripts/parity.py`
//! compares with the same EQ written in JUCE, is compared exactly with a
//! committed golden file.
//!
//! The outputs run to megabytes, so the golden file holds each mode's line
//! count, a hash of the whole output and its first lines, and a failure names
//! the mode that moved. `SEAM_BLESS=1` rewrites the file; the bench's parity
//! run then says whether the JUCE side still agrees.
//!
//! The coefficients use the C runtime's `powf`, `sinf`, `cosf` and `tanf`
//! (`src/dsp.rs`), so the golden file is exact for the runtime it was written
//! with, and the test runs on Windows only, the platform of the bench.

#![cfg(windows)]

use std::fmt::Write as _;
use std::path::Path;

#[path = "../examples/parity/cases.rs"]
mod cases;

fn run(mode: &str, stdin: &str) -> String {
    let mut out = Vec::new();
    match mode {
        "ranges" => cases::ranges(&mut out),
        "snap" => cases::snap(&mut out),
        "normalized" => cases::normalized(&mut out),
        "coefficients" => cases::coefficients_grid(&mut out),
        "audio" => cases::audio(stdin.as_bytes(), &mut out),
        _ => unreachable!("a mode the example has"),
    }
    String::from_utf8(out).expect("text")
}

/// How many lines of each mode the golden file shows, so a failure's diff
/// has something to read.
const SHOWN: usize = 3;

/// FNV-1a over the bytes: enough to tell any change from none, and the
/// same on every platform.
fn fnv1a(text: &str) -> u64 {
    text.bytes().fold(0xcbf2_9ce4_8422_2325, |hash, byte| {
        (hash ^ u64::from(byte)).wrapping_mul(0x0000_0100_0000_01b3)
    })
}

/// A seeded script for the `audio` mode: a rate, then host writes to random
/// parameters between blocks of many sizes, and a reset.
fn audio_script(keys: &[String]) -> String {
    let mut state = 0x2545_F491_4F6C_DD1D_u64;
    let mut next = move || {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        state
    };
    let mut script = String::from("rate 48000\n");
    for round in 0..300 {
        for _ in 0..next() % 4 {
            let key = &keys[usize::try_from(next() % keys.len() as u64).expect("small")];
            let unit = f32::from(u16::try_from(next() % 65_536).expect("16 bits")) / 65_535.0;
            writeln!(script, "set {key} {:08x}", unit.to_bits()).expect("write to a string");
        }
        if round == 150 {
            script.push_str("reset\n");
        }
        let frames = [1, 7, 16, 32, 64, 128, 333, 512, 1024, 4096]
            [usize::try_from(next() % 10).expect("small")];
        writeln!(script, "block {frames}").expect("write to a string");
    }
    script
}

fn golden() -> String {
    let ranges = run("ranges", "");
    let keys: Vec<String> = ranges
        .lines()
        .map(|line| line.split(' ').next().expect("a key").to_owned())
        .collect();
    let mut text = String::from(
        "# The parity example's output, mode by mode: lines, FNV-1a 64 of the whole output, first lines.\n",
    );
    let script = audio_script(&keys);
    for (mode, stdin) in [
        ("ranges", ""),
        ("snap", ""),
        ("normalized", ""),
        ("coefficients", ""),
        ("audio", script.as_str()),
    ] {
        let output = if mode == "ranges" {
            ranges.clone()
        } else {
            run(mode, stdin)
        };
        let shown = if mode == "ranges" { usize::MAX } else { SHOWN };
        writeln!(
            text,
            "{mode} lines={} fnv1a64={:016x}",
            output.lines().count(),
            fnv1a(&output)
        )
        .expect("write to a string");
        for line in output.lines().take(shown) {
            writeln!(text, "  {line}").expect("write to a string");
        }
    }
    text
}

#[test]
fn the_examples_ranges_snaps_writes_coefficients_and_audio_match_the_golden_file() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/golden/parity.txt");
    seam_testkit::drift::check_generated(&path, &golden());
}
