#!/usr/bin/env bash
# Runs the instruction-count benchmarks (iai-callgrind, under valgrind, so
# Linux). With a base revision, runs them there first, then at the current
# revision against that base, and fails if any instruction count grew by
# more than TOLERANCE.
#
# Usage: scripts/run-benches.sh [base revision]
set -euo pipefail

# Checking out the base replaces this file, which bash is still reading, so
# run from a copy.
if [[ -z "${RUN_BENCHES_COPY:-}" ]]; then
    copy="$(mktemp)"
    cp "$0" "$copy"
    RUN_BENCHES_COPY=1 exec bash "$copy" "$@"
fi

TOLERANCE="${TOLERANCE:-2%}"
# Every workspace crate with a bench target named iai runs it. The example EQ
# is built without its VST3 module, which does not build on Linux.
bench() {
    cargo bench --locked --workspace --no-default-features --features seam-example-eq/cabi --bench iai -- "$@"
}

base="${1:-}"
if [[ -z "$base" ]]; then
    bench
    exit 0
fi

# Whatever fails below, the checkout ends where it started: on its branch if
# it was on one, else at its commit.
start="$(git symbolic-ref --quiet --short HEAD || git rev-parse HEAD)"
head="$(git rev-parse HEAD)"
trap 'git checkout --quiet "$start"' EXIT
git checkout --quiet "$base"
if [[ ! -f crates/ump/benches/iai.rs ]]; then
    git checkout --quiet "$head"
    echo "the base has no benchmarks to compare with; running them alone"
    bench
    exit 0
fi
bench --save-baseline=base
git checkout --quiet "$head"
bench --baseline=base --callgrind-limits="ir=$TOLERANCE"
