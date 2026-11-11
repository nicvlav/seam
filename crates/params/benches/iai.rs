//! Instruction counts for a host's parameter write on the audio thread, one
//! kind of parameter at a time. Run by CI with iai-callgrind, which compares
//! them with the pull request's base.

#![allow(missing_docs, reason = "the benchmark macros generate public items")]

use std::hint::black_box;
use std::sync::Arc;

use iai_callgrind::{library_benchmark, library_benchmark_group, main};
use seam_params::{AudioParams, Kind, ParamDecl, ParamId, ParamSet, ParamSetBuilder};

const LABELS: [&str; 4] = ["a", "b", "c", "d"];

/// A set of one parameter of `kind`, and a handle to write it.
fn one(kind: Kind) -> (Arc<ParamSet>, AudioParams, ParamId) {
    let mut builder = ParamSetBuilder::new();
    builder
        .add(ParamDecl::new("p", "p", kind, 0.0))
        .expect("valid");
    let set = builder.build();
    let audio = set.audio();
    let id = set.ids().next().expect("one parameter");
    (set, audio, id)
}

#[library_benchmark]
#[bench::skewed(one(Kind::continuous(20.0, 20_000.0, 0.0, Some(1_000.0))), 0.37)]
#[bench::stepped_past_the_last_point(one(Kind::continuous(0.0, 10.0, 3.0, None)), 0.95)]
#[bench::bool(one(Kind::Bool), 0.8)]
#[bench::choice(one(Kind::Choice(&LABELS)), 0.6)]
fn host_write(
    state: (Arc<ParamSet>, AudioParams, ParamId),
    normalized: f64,
) -> (Arc<ParamSet>, AudioParams, ParamId) {
    let (_set, audio, id) = &state;
    audio.host_write(black_box(*id), black_box(normalized));
    state
}

library_benchmark_group!(name = params; benchmarks = host_write);
main!(library_benchmark_groups = params);
