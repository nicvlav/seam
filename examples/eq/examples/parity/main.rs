//! Writes what the example EQ stores and computes, for
//! `bench/scripts/parity.py` to compare with the same EQ written in JUCE
//! (`bench/`), before any audio is compared: one ulp of difference in a value
//! or a coefficient already breaks the comparison's bar at low frequencies.
//!
//! `cargo run --release -p seam-example-eq --example parity -- <mode>`, one
//! line per case on stdout, every number as its bits in hex:
//!
//! - `ranges`: each parameter's key, kind, start, end, interval, skew centre
//!   and default.
//! - `snap`: `key input snapped`, for every lattice point of every stepped
//!   range and the points a quarter, half and three quarters of a step past
//!   it, as a restore or the plugin's own write snaps them.
//! - `normalized`: `key normalized value`, a host's 0..1 through the host
//!   write, over 4,097 even steps and 4,096 seeded ones, each an `f32`
//!   widened, since a JUCE parameter takes its 0..1 as an `f32`.
//! - `coefficients`: `shape gain frequency q rate b0 b1 b2 a1 a2`, every
//!   lattice point of each of a band's parameters with the others at their
//!   defaults, and 2,000 seeded combinations, at 44.1, 48 and 96 kHz.
//! - `audio`: reads a script on stdin (`rate r`, then `set key n` for a
//!   host's 0..1 write, `block frames` and `reset`) and plays it through
//!   the plugin, both channels fed the same seeded noise as `bench-parity`
//!   makes, printing `left right` for every sample out.

mod cases;

use std::io::{BufWriter, Write};

fn main() {
    let mut out = BufWriter::new(std::io::stdout().lock());
    match std::env::args().nth(1).as_deref() {
        Some("ranges") => cases::ranges(&mut out),
        Some("snap") => cases::snap(&mut out),
        Some("normalized") => cases::normalized(&mut out),
        Some("coefficients") => cases::coefficients_grid(&mut out),
        Some("audio") => cases::audio(std::io::stdin().lock(), &mut out),
        _ => {
            eprintln!("usage: parity ranges|snap|normalized|coefficients|audio");
            std::process::exit(2);
        }
    }
    out.flush().expect("flush stdout");
}
