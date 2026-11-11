//! Instruction counts for the audio thread's parameter calls: a host's write,
//! one kind of parameter at a time, and the reads a block makes. Run by CI
//! with iai-callgrind, which compares them with the pull request's base.

#![allow(missing_docs, reason = "the benchmark macros generate public items")]

use std::hint::black_box;
use std::sync::Arc;

use iai_callgrind::{library_benchmark, library_benchmark_group, main};
use seam_params::{
    AnyParam, AudioParams, Changes, Float, Kind, ParamDecl, ParamId, ParamSet, ParamSetBuilder,
};

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

/// Four continuous parameters, bound, the audio thread's handle to their
/// set, and what a block last saw of them, as a `Changes` and as a sum of
/// counts. Each benchmark below hands it back, so its teardown falls outside
/// the count.
struct Four {
    audio: AudioParams,
    params: [Float; 4],
    changes: Changes,
    seen: u64,
}

fn four() -> Four {
    let decl = |key| ParamDecl::new(key, key, Kind::continuous(0.0, 1.0, 0.0, None), 0.0);
    let set = ParamSet::from_decls(["a", "b", "c", "d"].map(decl)).expect("valid");
    let params = ["a", "b", "c", "d"].map(|key| set.bind(key).expect("continuous"));
    let mut changes = Changes::new();
    let [a, b, c, d] = &params;
    assert!(changes.moved([a, b, c, d]), "the first check");
    Four {
        audio: set.audio(),
        params,
        changes,
        seen: 0,
    }
}

#[library_benchmark]
#[bench::float(four())]
fn get(state: Four) -> Four {
    black_box(black_box(&state.params[0]).get());
    state
}

// The check a block makes when nothing moved.
#[library_benchmark]
#[bench::four_members(four())]
fn moved(mut state: Four) -> Four {
    let [a, b, c, d] = &state.params;
    let members: [&dyn AnyParam; 4] = black_box([a, b, c, d]);
    black_box(state.changes.moved(members));
    state
}

// What `moved` replaces: four counts by id, summed by hand and compared.
#[library_benchmark]
#[bench::four_members(four())]
fn summed_by_id(state: Four) -> Four {
    let ids = black_box(state.params.each_ref().map(Float::id));
    let now = ids
        .iter()
        .fold(0_u64, |sum, &id| sum.wrapping_add(state.audio.changes(id)));
    black_box(now != state.seen);
    state
}

library_benchmark_group!(name = params; benchmarks = host_write, get, moved, summed_by_id);
main!(library_benchmark_groups = params);
