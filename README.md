# Seam

Declare a plugin parameter once, in Rust, and it is automation-correct,
undo-clean and persisted. VST3 sees it natively; AU, AAX and hosting see it
through a generic JUCE adapter driven by a generated C header. MIDI learn and a
typed React page are built on the same declaration, though neither reaches a
plugin in a host yet ([Status](#status)).

## Purpose

Seam makes Rust's ecosystem and tooling usable for audio plugins, so plugin code
is quicker to write well.

- **Performance.** A plugin's DSP runs comparably to the same DSP in C++.
- **Evidence.** `bench/` compares Seam against plain JUCE. With the same
  filters, compiled by LLVM inside the same JUCE wrapper, the two are within 3%
  of each other in 16 of the 18 block sizes and scenarios measured, plain JUCE
  level or ahead. In the other two, at 512 frames with parameters moving, a
  build's three timed runs disagree by up to 29%. The two builds' medians there
  differ by 9% one way and 6.5% the other ([the ladder
  report](bench/reports/2026-11-05-ladder.md)).

Seam succeeds the C++/JUCE Seam (`nicvlav/seam-cpp`, final release v0.7.0).
None of its code came over; its contracts did, in
[docs/transfer-from-seam-juce.md](docs/transfer-from-seam-juce.md).

## Status

Pre-release: 0.1.0, unpublished. Seam follows [semantic
versioning](https://semver.org/): before 1.0, a minor version may break the API.
[CHANGELOG.md](CHANGELOG.md) says what each version holds.

- **Built:** the core crates, the VST3 exporter (Windows and macOS), the C
  interface and the generic JUCE adapter, the page's React hooks and protocol,
  the example gain, the example EQ, and the comparison against plain JUCE.
- **Not built:** the CLAP exporter; the native side that answers the page; the
  WebView glue that opens the page in a plugin window (wry's IPC handler, the
  custom-scheme asset server), which is to be a shell crate so `seam-bridge`
  stays free of any WebView; and MIDI delivered to a plugin through an
  exporter. [ROADMAP.md](ROADMAP.md) says what comes next.

## Start here

[docs/first-plugin.md](docs/first-plugin.md) walks through `examples/gain`, a
small plugin that uses parameters, undo, saved state, MIDI learn and the VST3
export together. Read it first, then the crate overviews it links.

## Layout

The Rust crates are in `crates/`: the core (`params`, `undo`, `session`,
`learn`, `ump`), the [exporters](docs/glossary.md#exporter) (`host`), the page
protocol (`bridge`) and test tools (`testkit`). Beside them are the example
gain (`examples/gain`), the example EQ (`examples/eq`), the page's React hooks
(`page`), the generic JUCE adapter (`adapters/juce`), the repository's tasks
(`xtask`), the fuzz targets (`fuzz`) and the comparison against plain JUCE
(`bench`). The codemap in [ARCHITECTURE.md](ARCHITECTURE.md#codemap) says what
each one is.

[ARCHITECTURE.md](ARCHITECTURE.md) has the codemap, the invariants and the
thread model, and [docs/testing.md](docs/testing.md) the test rules and what
each CI lane proves. The [style guide](docs/style.md) says how code and docs
are written here and lists the other docs: the decision record, its index and
history, and the [glossary](docs/glossary.md) of the words Seam coins.

## Develop

The minimum supported Rust version is 1.99, the workspace's `rust-version`;
`rust-toolchain.toml` pins 1.99.1 for development. The page needs Node 22.22.2,
24.15.0 or 26 and later (jsdom 30's floor).

```sh
cargo fmt --all --check
cargo test --workspace --all-features
cargo clippy --workspace --all-targets --all-features -- -D warnings
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --all-features --no-deps
cargo xtask wrap --max 80
python3 scripts/check-core-deps.py
python3 scripts/check-comment-tags.py
python3 scripts/check-decision-index.py
cd page && npm ci && npm run typecheck && npm test
```

These are the checks a change most often trips. Run the scripts with `python3`
(`python` on Windows), from a git checkout with `git` on the path: the comment
check reads the files git tracks. CI runs more, the release allocation gates
and the loom models among them: [docs/testing.md](docs/testing.md) lists every
lane and what it proves.

## Licence

MIT ([LICENSE](LICENSE)). A plugin built with Seam also carries the VST3 SDK's
MIT notice and `triple_buffer`'s MPL-2.0 notice, in
[THIRD-PARTY-NOTICES](THIRD-PARTY-NOTICES).
