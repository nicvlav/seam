# Testing

Every behaviour change carries a test. A bug fix starts with a test that fails
for the bug, seen failing before the fix and passing after it. Every test file
starts with a `//!` line that says what it tests: a test crate is a crate, and
`missing_docs` fails one without that line under CI's `-D warnings`. Each
command below is marked with where CI runs it ([CI lanes](#ci-lanes)): on a
pull request, in the full run, or in the nightly or weekly workflow. All but
the pull request's are started by hand.

```sh
cargo test --workspace --all-features                                # pull request: everything, debug
cargo test --workspace --all-features --release --test alloc_gate   # pull request: allocation gates, optimised
cargo test --workspace --all-features --release                      # full run: everything, optimised, doc tests included
cargo clippy --workspace --all-targets --all-features --release -- -D warnings   # full run: with debug assertions off
python3 scripts/check-core-deps.py   # pull request; `python` on Windows, where python3 is the Store stub
cargo test -p xtask --no-default-features   # pull request: the Rust reader the next two checks use
python3 scripts/check-comment-tags.py                                # pull request
python scripts/check-example-coverage.py   # full run; needs a nightly compiler (NIGHTLY names it)
python3 scripts/check-decision-index.py                              # pull request
python3 scripts/test-checks.py       # pull request: the check scripts refuse their bad input
python3 scripts/check-shape-append-only.py origin/main   # pull request, against its base: the shape file only gains lines
cargo xtask wrap                     # pull request: the comment lines of crates/, examples/ and xtask/ past 80 columns
cargo xtask wrap --markdown          # pull request: the prose lines of the repository's docs past 80 columns
RUSTFLAGS="-Zsanitizer=realtime --cfg seam_rtsan" cargo +nightly test ... --target x86_64-unknown-linux-gnu   # nightly workflow: see Real-time sanitizer
cargo test --manifest-path fuzz/Cargo.toml --lib   # nightly workflow: the fuzz crate's unit test
python3 scripts/run-loom.py          # pull request: the loom models, counted
cargo test -p seam-example-eq --release --test matrix --test overflow   # pull request: the gate matrix
SEAM_SOAK_SECONDS=60 cargo test -p seam-example-eq --release --test matrix -- --ignored soak --nocapture   # nightly workflow (30 minutes) and weekly workflow (3 hours)
cargo +nightly fuzz run saved_block  # nightly workflow, ten minutes per target; in fuzz/, needs a nightly compiler
scripts/run-benches.sh               # pull request, against the base; Linux, with valgrind and iai-callgrind-runner
cd page && npm ci && npm run typecheck && npm test                   # pull request
```

## CI lanes

`.github/workflows/ci.yml` runs in two sizes.

**A pull request** takes a thin gate, in about three minutes: what a
developer's own Windows machine cannot show, and the checks that cost seconds.
Those are `linux`, the macOS leg of `rust` (two steps: Clippy with all
features, and Test), `core builds alone`, `loom models`, `benchmarks`, `page`,
`seam.h` and `protocol shape history`. It runs once the pull request is ready
for review. A draft runs nothing, and `main` is not tested again after a
merge: the squash of a branch rebased onto `main` is the tree its pull request
tested. So before a pull request, the commands above marked for it are run on
the developer's machine, where they cover Windows.

**The full run** is the same workflow started by hand, from any branch. It
takes every lane in the table, the Windows leg of `rust`, every step of both
legs, and the larger property-test count. Start one for a change the thin gate
cannot judge, such as one to the exporters or to release-only code, and for
the commit a release is cut from.

The JUCE adapter's workflow (`juce-adapter.yml`) and the long lanes
(`nightly.yml`, `weekly.yml`) are started by hand too, and have no schedule
yet, so the runner minutes go to the pull request gate. Each long lane's
command is in the list above and runs on a developer's machine, except the
real-time sanitizer, which needs Linux.

| Lane | What it proves |
| --- | --- |
| `rust` (Windows, macOS): Format | The code is formatted. |
| `rust`: Clippy, default features and all features | No warning, including the workspace's doc and panic lints, with each feature set. |
| `rust`: Test | Every test passes in debug, where contract assertions fire: the drift gates, the allocation gates and the example EQ's golden-file parity test included. |
| `rust`: Clippy (release) | No warning with debug assertions off, where the code under `cfg(debug_assertions)` drops out. |
| `rust`: Test (release) | Every test and doc test passes in optimised code with no debug assertions, which is what a host runs: the audio-thread mark's release half and the doc examples that read it included. |
| `rust`: Allocation gates (release) | The audio path still does not allocate in optimised code, which is what a host runs. |
| `rust`: Gate matrix (release) | The example EQ through both exporters allocates nothing and equals the processor driven directly, across block sizes, sample rates and layouts (section 2). |
| `rust`: Docs | Every doc builds with no warning, broken link included. |
| `doc examples` (Windows, pinned nightly) | Every struct, enum, trait, function and macro a user can reach has a Rust example, in every library crate under `crates/` (see [Doc examples](#doc-examples)). |
| `linux` | The code is formatted, and the core, the bridge, the C interface and the example EQ through it build, pass clippy and pass their tests, gates and matrix on Linux, and the VST3 module refuses to compile there, with its message. |
| `seam.h` | The header compiles as C11 and C++11 for x86_64 and aarch64, so its size and offset assertions hold for both. |
| `cross check` (aarch64 Linux, aarch64 Android) | The core, the bridge and the C interface type-check for those targets. Nothing is linked or run. |
| `cargo deny` | Every dependency of the workspace has a licence `deny.toml` lists, no advisory applies (two, in the benchmark harness, are named there), no source is outside crates.io. It does not read `fuzz/`, which is outside the workspace and whose libFuzzer is under NCSA. |
| `coverage` | Line coverage of the code Linux builds is at least 92 % (see [Coverage](#coverage)). |
| `plugin validators (windows)`: pluginval | pluginval 1.0.4, pinned by hash, passes the example EQ's VST3 bundle at strictness level 5 without a GUI, run in the validator's own process. It does not show what a real host does with it. |
| `plugin validators (windows)`: The Steinberg validator | The Steinberg validator, built from the VST3 SDK at a pinned commit, passes the same bundle. It runs whether or not pluginval passed. |
| `benchmarks` | On a pull request, no benchmark's instruction count grew more than 2 % against the base (see [Benchmarks](#benchmarks)). |
| `core builds alone`: No window, WebView or host dependency | No core crate or `seam-bridge` links a window, WebView, UI toolkit or plugin-format crate, and no crate depends on one of its own layer or above (see [Repository checks](#repository-checks)). |
| `core builds alone`: The Rust reader's tests | `cargo xtask scan`, `defs` and `wrap` read comments, string literals, `fn` names and item definitions as the language does (see [Repository checks](#repository-checks)). |
| `core builds alone`: Comments wrap at 80 columns | No `///`, `//!` or `//` prose line of `crates/`, `examples/` or `xtask/` passes 80 columns (see [Repository checks](#repository-checks)). |
| `core builds alone`: Docs wrap at 80 columns | No prose line of README.md, ARCHITECTURE.md, CHANGELOG.md, `bench/README.md`, `page/README.md` or `docs/*.md` passes 80 columns (see [Repository checks](#repository-checks)). |
| `core builds alone`: No decision ids in comments or docs | No comment or doc in a tracked file cites a decision id, an audit finding id or a stage, and no code comment points at the record (see [Repository checks](#repository-checks)). |
| `core builds alone`: The decision index matches the record and the code | Every file and item `docs/decision-index.md` lists exists, and it has a heading for every id in the decision record. |
| `core builds alone`: The check scripts' own tests | Each check script refuses the bad input it exists for (see [Repository checks](#repository-checks)). |
| `core builds alone`: Build the core alone | Every workspace crate but the host, the two example plugins, the test tools and xtask builds on a runner with no WebView or GTK libraries; a new crate is built there until it is excluded. |
| `protocol shape history` (pull requests) | `crates/bridge/protocol-shape.txt` only gains lines against the base revision (see [Repository checks](#repository-checks)). |
| `loom models` | The lock-free hand-offs are correct under every interleaving their atomics allow, and `scripts/run-loom.py` fails unless every model ran (section 3). |
| `page`: Typecheck, Test | The page compiles against the committed generated types, and its tests pass. |
| `JUCE adapter` (macOS, started by hand) | The example EQ builds through the adapter as VST3 and AU, at the JUCE commit `bench/` pins, and both the Rust library and the plugin binaries hold arm64 and x86_64. It does not load the plugin. |
| `JUCE adapter` (Windows, started by hand) | The example EQ's VST3 build through the adapter passes the Steinberg validator, built from the VST3 SDK at the pinned commit, as the native `plugin validators (windows)` lane passes Seam's own. |
| Nightly: `miri` | `seam-host`'s tests, the core crates' unit tests and some integration tests find no undefined behaviour under Stacked Borrows and Tree Borrows (see [Miri](#miri)). |
| Nightly: `rtsan` (Linux) | A canary that allocates inside a marked call aborts, and the allocation gates and the gate matrix pass under RealtimeSanitizer. Not run yet; see [Real-time sanitizer](#real-time-sanitizer). |
| Nightly: `fuzz` | Each of four `cargo-fuzz` targets ran ten minutes with no crash, no panic and no broken fixpoint (see [Fuzzing](#fuzzing)). |
| Nightly: `soak` (Linux, Windows) | The matrix's conditions, drawn at random for 30 minutes, with random parameter moves, produced no allocation, fault, NaN, denormal or drift from the processor driven directly. |
| Weekly: `soak` | The same for 3 hours. |

A plugin crate builds the VST3 module by default, which refuses to build on
Linux. So a plugin crate added to the workspace joins the `--exclude` lists of
the steps that build default features there: the core-alone build and the
cross check in `ci.yml`, and the miri run in `nightly.yml`.

The rules below each close a class of failure that reaches a release. Their
tools live in `crates/testkit`, except the boundary record, which lives with the
exporters in `seam-host`.

## 1. A contract assertion is a test failure

Library code checks its callers' contracts with `debug_assert!`. Tests build
with debug assertions, except the release run of the allocation gates, which has
none. So in every other run a violated contract panics. A panic on the test
thread, or on a thread the test joins, fails the test. An assertion can never
pass as a line in a log.

What keeps that true:

- **No detached threads.** A test runs its threads in `std::thread::scope` or
  joins them, so their panics reach it. Library code never detaches a thread
  either. A panic nobody joins is a failure nobody sees. One test leaves a
  thread behind, and only when it fails: `crates/params/tests/drops.rs` runs
  each drop on a worker it gives up on after a bound, since the regression it
  pins is a deadlock.
- **A deliberate violation is declared.** A test that breaks a contract on
  purpose wraps the call in
  `seam_testkit::contract::expect_contract_panic(expected, f)`. That fails when
  nothing fires, and when the panic's message does not contain `expected`. An
  assertion removed from the library fails its test instead of silently passing
  it. An unrelated panic cannot stand in for the contract under test. Such a
  test is `#[cfg(debug_assertions)]`, since release builds compile the assertion
  out.
- **Boundaries record what they catch.** A host calls in through COM or the C
  ABI, where a panic must not unwind. The exporters catch it there and count it,
  and a call refused for its thread the same way, per instance and per process
  (`seam_host::boundary_faults()`). `crates/host/src/faults.rs` says when the
  first fault of each kind is written as a line; in a test build of `seam-host`
  the line goes to a list the test reads instead.

  `seam_host::vst3::TestHost` drives a plugin through the VST3 exporter's own
  COM interfaces. `seam_host::cabi::TestHost` drives it through the C
  interface's table, as the JUCE adapter calls it. A call the plugin may refuse
  answers `Result<(), Refused>`: the VST3 host's `Refused` carries the
  `tresult`, the C interface's stands for its `false`, so a test pins what the
  host was told. Both exist only with `seam-host`'s `test-host` feature, which a
  plugin crate turns on in its dev-dependencies, so a shipping plugin carries
  neither. When dropped, each checks that its instance had no fault on any
  thread, less the faults the test declares with `expect_faults`. A test that
  provokes one on purpose declares it and checks that the count moved by exactly
  one.

## 2. The audio path does not allocate

Code the audio thread runs never allocates, frees, locks or makes a system call.
Every buffer it touches has a fixed capacity, sized when the plugin is activated
on the main thread and never grown.

Allocation is checked by test. Each crate with audio-thread code has a test
target named `alloc_gate` (`tests/alloc_gate.rs`): `seam-host`, `seam-learn`,
`seam-params`, `seam-ump`, and the example EQ and gain. `seam-testkit` has one
too, which checks the gate itself. Each:

1. installs the counting allocator once: `seam_testkit::install_alloc_gate!();`
2. builds and activates what it tests outside the gate, where allocating is
   allowed;
3. runs the audio-thread path inside `assert_no_allocations("what", || ...)`,
   including the first block after activation: nothing is allowed to warm up on
   the audio thread.

The count is the calling thread's own, so the gated code runs on the thread that
calls the gate. The gate proves it is installed before trusting a zero. A test
binary that forgot `install_alloc_gate!` fails rather than passing every gate.

The gate sees allocations only. Locks and system calls are kept off the audio
path by review. In `process` and anything it calls there is no `Mutex`,
`RwLock`, channel from `std::sync::mpsc`, `println!`, file access or thread
spawn. Nor is there an `Arc` drop that could be the last one. Wait-free queues
and atomics cross threads.

A debug build also checks the locking half. `seam_params::audio_thread::enter()`
marks the calling thread as the audio thread until its guard drops. These assert
the mark is absent:

- every call that locks (`Value`, `List`, `Session`, `Load::restore`, the load
  hold that undo replays take);
- the handles a set hands out once (`ParamSet::take_ui`,
  `ParamSet::take_host_event_drain`);
- the drops of `AudioParams`, `LearnAudio` and `HostEventDrain`.

An exporter holds the guard around everything its process call runs. So every
test that drives the plugin through an exporter fails at the first lock on the
audio path. This proves only what the tests execute. It cannot see a lock on a
path no test reaches, a lock taken inside a dependency, or a system call. In a
release build there is no mark and no check:
`crates/params/tests/audio_thread.rs` pins both, and `rust`: Test (release)
runs it there. No CI lane has yet checked system calls mechanically; the
nightly `rtsan` lane is written to (see [Real-time
sanitizer](#real-time-sanitizer)).

The audio-path modules also refuse some clippy restriction lints, by `deny`
attributes on the module or the function. The lints are `indexing_slicing`,
`unwrap_used`, `expect_used`, `panic` and `arithmetic_side_effects`. The modules
are the UMP readers and event list, learn's dispatch, and the parameter store
and its host event queue. The rest are the exporter core's process path,
automation walk and channel staging (`crates/host/src/boundary/channels.rs`),
and the example EQ's filters and block. A site that cannot fail says why in an
`#[expect(..., reason = "...")]`.

`examples/eq/tests/matrix.rs` runs the example EQ through both exporters under
the allocation gate. It runs in debug and release, with the audio-thread mark
held. The matrix covers:

- eight block sizes (1, 2, 31, 32, 64, 512, 4,096 and 8,192 frames);
- six rates (44.1, 48, 88.2, 96, 176.4 and 192 kHz);
- every layout;
- every parameter moving, with up to 12 points per block (more than the exporter
  has room for on average);
- silent blocks, to let filters ring out.

Each block must equal the processor driven directly after the same last points,
sample for sample. The exception is below -400 dBFS, where the exporters flush
denormals and the direct processor does not. `overflow.rs` sends far more points
than the exporter's room, so the walk's room-full branches run. The exporters
hand a plugin no MIDI yet, so the matrix fills the event lists to capacity on
the processor-driven side only. `matrix.rs`'s `soak` test, ignored in a normal
run, plays the matrix at random for `SEAM_SOAK_SECONDS` and prints the seed it
used (`SEAM_SOAK_SEED` repeats a run).

The example EQ's parity output must also equal its golden file. That test runs
on Windows only.

### Real-time sanitizer

RealtimeSanitizer (`rtsan`) aborts a thread that allocates, locks or makes a
blocking system call while inside a function marked nonblocking. It is the
mechanical check of the half the allocation gate cannot see.

What the pinned nightly (`nightly-2026-11-04`, rustc 1.101.0-nightly) supports:

- `-Zsanitizer=realtime` is accepted for `x86_64-unknown-linux-gnu`, where it
  sets `cfg(sanitize = "realtime")`. A Windows target refuses it as not
  supported. On Windows the lane is therefore only compiled, for the Linux
  target: add it with `rustup target add`, then run the lane's commands as
  `cargo check`, which links nothing.
- A function is marked with `#[sanitize(realtime = "nonblocking")]`. The other
  values are `"blocking"` (a function that is allowed to block, called from a
  nonblocking one) and `"caller"` (take the caller's). The attribute needs
  `#![feature(sanitize)]` and is unstable (tracking issue 39699), so a stable
  build must never see it.

What the lane does: `nightly.yml`'s `rtsan` job builds for
`x86_64-unknown-linux-gnu` with
`RUSTFLAGS="-Zsanitizer=realtime --cfg seam_rtsan"`. `--target` keeps the flags
off build scripts and proc macros.
First comes the canary, `crates/host/tests/rtsan_canary.rs`, built only under
the cfg: a processor that allocates, played through the C interface's marked
`process`. The step expects the test to abort with a RealtimeSanitizer report,
and fails the lane when it passes, so the lane cannot pass while checking
nothing. Then the lane runs the release allocation gates and the gate matrix,
as the `linux` lane does.

Marking is for `crates/`, behind the custom cfg so stable never sees the
attribute: `#![cfg_attr(seam_rtsan, feature(sanitize))]` at a crate root, and
`#[cfg_attr(seam_rtsan, sanitize(realtime = "nonblocking"))]` on a function.
The workspace's `unexpected_cfgs` setting names `cfg(seam_rtsan)`. The
sanitizer checks everything a marked function calls, so the marks go on the
audio thread's entry points. These carry it, in `seam-host`:

- `Engine::process` and `Engine::play` (`crates/host/src/engine/process.rs`),
  which every exporter's process goes through;
- the C interface's `process`, `get`, `set` and `reset`
  (`crates/host/src/cabi/mod.rs`) and the VST3 exporter's `process` and
  `setProcessing` (`crates/host/src/vst3/component.rs`): the calls that carry
  the audio-thread mark. The VST3 exporter does not build for Linux, so its
  marks are compiled on Windows and macOS and never run under the sanitizer.

The parameter store's audio-side reads and writes (`crates/params/src/set.rs`,
`host_events.rs`), learn's audio side (`crates/learn/src/audio.rs`) and the UMP
crate's event list and readers (`crates/ump/src/lib.rs`, `packets.rs`,
`voice.rs`) carry marks of their own. The lane runs every crate's allocation
gate, so the gates of the parameters, learn and UMP crates check those marks
under the sanitizer too, as well as through the exporters. The plugin's
processor runs below the exporters' marks in the lane's tests. A processor's
`process` is a trait method, so a plugin would mark its own impl, as the
example EQ's block could.

A panic on the audio path allocates its payload, and std's hook writes it to
stderr, inside a marked call, so the sanitizer reports it. It happens only
after a fault, and the report is the right outcome.

Open: unverified until CI runs it. The sanitizer's runtime must ship with the
nightly for this target, which Windows cannot show; the first run settles it.
The sanitizer sees only the paths the tests run, and only on Linux and macOS.

## 3. Concurrency is tested on real threads, without sleeps

A test of a cross-thread contract runs real threads and makes them meet. One
side counts a `Latch` down and the other waits on it, or a side waits for a
condition with `wait_until`. A fixed sleep never stands in for an ordering.
Timeouts (`PATIENCE`, ten seconds) only bound a hang; running out fails the
test.

A test waits a fixed time only where there is nothing to wait for: it checks
that something does not happen, or it runs a real timer.

- `a_load_waits_while_a_replay_holds_loads_off`
  (`crates/undo/tests/restore.rs`) waits 20 ms to check that a load has not yet
  written while a replay holds loads off. The deterministic pin of that side is
  the unit test in `crates/undo/src/history/tests.rs`.
- The timer's own tests (`crates/host/src/engine/timer/tests.rs`) and the VST3
  exporter's timer tests (`crates/host/src/vst3/tests.rs`) run the thread's
  message loop or run loop for a set time (`pump`), so a real timer ticks, and
  then check that no tick comes after it stops. On Windows, `stopped`
  (`engine/timer.rs`) waits four timer periods before it asks whether a
  message is due.
- `a_host_on_three_threads_with_real_ticks_and_points_at_offsets`
  (`crates/host/src/vst3/tests.rs`) paces its audio thread's blocks 250 µs
  apart while it waits, ten seconds at most, for one to hear the main thread's
  tick.

Every wait is bounded. A slow machine can only make a check that something does
not happen pass wrongly, never fail wrongly; the one test that counts ticks
asks for 5 in 400 ms, where about 25 are due.

The exporters' test hosts drive an instance as impolitely as real hosts do:

- It is created on one thread and initialized or first called on another
  (`created_elsewhere`).
- Blocks are sent from an audio thread of its own (`audio_side`) while the
  test's thread makes main-thread calls.
- Automation arrives as queues of points at sample offsets (`process_queues`).
- The VST3 exporter's own timer ticks through the thread's message loop or run
  loop (`with_timer`, `pump_until`).

The rules the exporters share are tested through both test hosts with one body
each (`crates/host/src/engine/tests.rs`). The run loop timer exists only on
macOS, so its test runs only in CI's macOS job.

Two tools cover what real threads cannot make happen on demand:

- **A racing test double.** A test double pins the narrowest interleaving a
  contract allows. It performs the competing write at an exact point inside the
  code under test, such as between a read and its return. It does so on one
  thread, with no timing.
- **A model checker.** A lock-free protocol gets a `loom` model that explores
  every interleaving its atomics allow. `scripts/run-loom.py` finds the models
  in every Rust file of the repository and counts them, by crate, module and
  name, and fails unless each one ran. A model outside the `src/` of the
  lane's crates (params, learn and host) fails it too, since the lane would
  never run it. This list says what each checks, and a new model adds a line:
  - learn's staging slot: the newest staged value is the last one taken
    (`crates/learn/src/target.rs`);
  - learn's capture staging: a poll racing two blocks, an NRPN high byte and
    then its low byte, never settles the high byte as coarse, and weakening the
    block counts' or the staged word's release or acquire fails it
    (`crates/learn/src/capture/tests.rs`);
  - the parameters' host event queue: a drain racing a gesture hears one
    balanced bracket (`crates/params/src/host_events.rs`);
  - the parameters' count-then-value read: a read never has a count newer than
    its value (`crates/params/src/set/tests.rs`);
  - the parameters' store under two writers: a user's write on the UI thread
    races a host write on the audio thread. The race leaves one writer's value
    with that writer's origin, since the two are one atomic word. A read during
    the race returns one writer's value with that writer's origin
    (`crates/params/src/set/tests.rs`);
  - the parameters' writes with a step between store and publish: a user's
    write and a sync race a reader that compares counts, takes the re-read
    request and drains the host queue. The reader never shows a count or a
    request newer than the value, and the store always holds at least the value
    the host heard (`crates/params/src/set/tests.rs`);
  - the exporters' audio slot: a block racing an activation sees one whole
    processor. A panicking block racing one marks only its own processor
    (`crates/host/src/boundary/loom_models.rs`,
    `crates/host/src/engine/process.rs`);
  - the exporters' reset request: a reset asked once a block ran on the
    processor an activation installs is kept, since the activation clears the
    old request while it holds the slot (`crates/host/src/engine/process.rs`);
  - the exporters' main-thread state bound to no thread: a tick that tries once
    and a call that waits by trying again never touch it at once. The call
    borrows it a second time inside its loan and lets the outer loan go first.
    The waiting call always gets it, and it is free once both are done
    (`crates/host/src/boundary/loom_models.rs`);
  - the same state shared by two threads: it drops on whichever thread lets it
    go last, after both loans end, and exactly once
    (`crates/host/src/boundary/loom_models.rs`).

  A model fails when the ordering that publishes its data is weakened, with one
  exception: the queue's `dirty` flag. Its release and acquire are needed under
  the memory model, but loom's read-modify-writes always read the newest store,
  so the model cannot see them. They rest on the argument written at
  `HostQueue::mark_dirty` (`crates/params/src/host_events.rs`).

The `loom models` lane runs `RUSTFLAGS="--cfg loom" cargo test -p seam-params -p
seam-learn -p seam-host --release --locked --lib loom_`. The host's models need
no feature, since its core builds without one. The cfg swaps the modelled
atomics for loom's, which work only inside a model, so the lane runs the models
alone (the `loom_` prefix). `loom` is pulled in only under `cfg(loom)`.

### Miri

`miri` checks the `unsafe` code at the boundary: the exporters' tests drive host
objects written in Rust, and the system timer and the denormal flush stand down
under `cfg(miri)`. The host crate's tests pass under both of Rust's aliasing
models:

```sh
MIRIFLAGS="-Zmiri-ignore-leaks" cargo +nightly miri test -p seam-host --features vst3,cabi,test-host --lib
MIRIFLAGS="-Zmiri-ignore-leaks -Zmiri-tree-borrows" cargo +nightly miri test -p seam-host --features vst3,cabi,test-host --lib
```

`-Zmiri-ignore-leaks` is needed because some tests leak an instance on purpose.
A release or destroy on another thread, or from a host callback inside a tick,
must leak what a tick may still reach. Six tests are ignored there: five need a
real timer and one reads 64 MiB of stream, too slow to interpret. The channel
staging's model test, which tries 86,730 ways a host can hand its buffers, runs
every 863rd of them, about a hundred, under miri.

The nightly workflow runs this under both models. It also runs the core crates'
unit tests, the UMP crate's integration tests and `seam-learn`'s capture,
dispatch, saved and targets tests, with `-Zmiri-disable-isolation` and four
property cases. Learn's four take about 20 seconds under either model on
Windows. Its ceiling test maps 4,096 sources, did not finish in 25 minutes and
is skipped. Its allocation gate is not run (it hits the allocator fault
described below on Windows, and Linux has not run it yet). The other integration
tests are left out: `seam-undo`'s do not finish in an hour under the
interpreter.

No allocation gate runs under miri, so the counting allocator itself is not
checked for undefined behaviour. Open: whether the gates pass under miri on
Linux, where the nightly lane runs; nobody has tried. On Windows, std's `System`
allocator under a forwarding `#[global_allocator]` reads a header outside the
pointer's provenance for over-aligned blocks
(`library/std/src/sys/alloc/windows.rs`). A repro with no Seam code fails the
same way, so the fault is not Seam's.

## 4. Generated files do not drift

The page is typed from TypeScript generated from the Rust types, and the JUCE
adapter builds against a C header and per-plugin CMake settings generated the
same way. The generated files are committed, so a reader, a reviewer and the
page's and the adapter's own builds see them without running Rust. A drift gate
fails when the Rust types change and the committed file was not regenerated.
`xtask/tests/drift.rs` compares every file `cargo xtask gen` writes with the
committed one byte for byte, after reading CRLF line endings as LF.
`crates/bridge/tests/typescript_drift.rs` checks the protocol module on its own.
For the C header that covers every type, since each field's C declaration is
spelled from its Rust type. The header's own size and offset assertions then
fail a C++ build whose layout disagrees with the library's.

To regenerate, run the generator, review the diff and commit it with the Rust
change that caused it:

```sh
cargo xtask gen
```

Running a failing gate with `SEAM_BLESS=1` set (empty or `0` counts as unset)
rewrites its file too. With `CI` set the gate refuses to rewrite and fails, so a
pipeline cannot bless its own output.

CI then type-checks the page against the committed file, so a Rust change that
breaks the page fails in the same pull request. `.gitattributes` keeps text
files LF on every platform so the comparison is exact.

The generated files are:

- the protocol module;
- a parameter map for the page's own tests (`page/test/generated/params.ts`,
  from the declarations in `xtask/src/generate.rs`);
- the C header the `cabi` feature generates for the JUCE adapter
  (`crates/host/include/seam.h`);
- the example EQ's JUCE settings (`examples/eq/seam-juce.cmake`).

## 5. A check that finds nothing fails

A check that passes because it looked at nothing is worse than none. So the
checks listed here fail when they find nothing to check, and when their input is
not in the format they parse:

- `run-loom.py` fails when it finds no model, when a model lacks the `loom_`
  prefix its filter needs, when a model lies where the lane does not run it,
  and when a model in the source did not run;
- `check-comment-tags.py` and `check-decision-index.py` fail when the decision
  record yields no ids, and the index when it has no heading;
  `check-comment-tags.py` also when no tracked Rust file reaches the Rust
  reader, and `cargo xtask scan` and `defs` when handed no path or a file that
  does not lex or parse;
- `check-example-coverage.py` fails when a crate has no reachable public item,
  and when rustdoc's JSON is a format version it was not written for;
- `cargo xtask wrap` fails when `crates/`, `examples/` or `xtask/` holds no
  Rust file;
- `check-core-deps.py` fails on a workspace crate it has no layer for, and on a
  table entry that is no crate;
- `check-shape-append-only.py` fails on a shape file with no version line, in
  either revision;
- the drift gate fails when `cargo xtask gen` writes no file, and the allocation
  gate when the counting allocator is not installed;
- `crates/bridge/tests/fixtures_messages.rs` fails on a fixture list with no
  entry.

`scripts/run-benches.sh` is the exception: when the base revision has no
benchmarks it says so and runs the current ones alone, so that comparison has
nothing to compare. A new check states in its first lines what input it reads
and fails on an empty or foreign one. `scripts/test-checks.py` holds a test of
that for each script above that reads input.

## Repository checks

What each check in the `core builds alone` lane, and the protocol shape check,
fails on.

- **Core dependencies** (`scripts/check-core-deps.py`). No core crate or
  `seam-bridge` links a window, WebView, UI toolkit or plugin-format crate, on
  any platform, with any features. No crate depends on one of its own layer or
  above. The layers are the table in the script, checked against `cargo
  metadata`: a workspace crate missing from it, or an entry that is no workspace
  crate, fails. A crate under `examples/` is a plugin and needs no entry. Undo
  and session never depend on each other.
- **The Rust reader** (`xtask/src/scan.rs`, tested in `xtask/tests/scan.rs`).
  The two checks below do not tokenise Rust. `cargo xtask scan` lexes it with
  proc-macro2, so a string over several lines, a raw, byte or C string with any
  prefix, a char literal, a lifetime and a nested block comment are read by the
  language's own rules. A comment is the source between two tokens, and a doc
  comment is a token. `cargo xtask defs` parses the file with syn and lists the
  names it defines. Both print one fact per line for the Python scripts, which
  keep the Markdown, YAML, shell, TOML, Python and C-family files. A file that
  does not lex fails the check. The lane builds them with
  `--no-default-features`, which leaves out the generators and their crates.
- **Decision ids** (`scripts/check-comment-tags.py`). No comment or doc in any
  tracked file, outside the decision record, its index, its history
  (`docs/decision-history.md`), the dated bench reports and generated files,
  cites a decision id, an audit finding id or a stage, and no code comment
  points at the record. The same goes for the text of a Rust string literal (an
  assertion message), but only for ids the record defines in the form a comment
  would cite them (a letter and its number, not a bare number), and for a Rust
  `fn` name, which is checked for such an id as a word between underscores. A
  test name in TypeScript is not scanned, nor is an id written some other way.
  Shell scripts are read as comments too. A comment states its reason in words.
- **The decision index** (`scripts/check-decision-index.py`). Every file
  `docs/decision-index.md` lists exists and defines every item listed in it (a
  `fn`, type, `const`, `mod`, macro, a name in an `impl` header, or a named
  field or variant, as `cargo xtask defs` parses them; not a word in a comment,
  a `use` list or a struct literal), and it has a heading for every id in the
  decision record.
- **Prose width** (`cargo xtask wrap [--markdown] [--max N]`, tested in
  `xtask/tests/wrap.rs`). Lists each `///`, `//!` and `//` line of `crates/`,
  `examples/` and `xtask/` longer than N columns (80 by default, which rustfmt
  also uses for comments) and a count per package. Code in a fenced or indented
  example, table rows and lines holding a URL are exempt. `--markdown` lists
  the prose lines of README.md, ARCHITECTURE.md, CHANGELOG.md,
  `bench/README.md`, `page/README.md` and `docs/*.md` past N columns instead;
  fenced code, headings, tables, link definitions, a line holding a word or
  link too long for a line of its own, and the decision index's site lines are
  exempt. CI runs both with `--max 80` in `core builds
  alone`. Open: an example that pulls in a fragment through `#[doc =
  include_str!(..)]` reads as prose after that line, so its code is held to the
  width too.
- **The scripts' own tests** (`scripts/test-checks.py`). Each check script
  refuses the bad input it exists for:
  - an id of three digits;
  - a record with no ids;
  - a `'"'` char literal before a comment in C and TypeScript;
  - a crate outside `examples/` with no table entry;
  - a rustdoc format it does not know;
  - same-named loom models in two crates;
  - cargo's coloured output;
  - a loom source with no model;
  - an index with no heading;
  - a Rust comment after a string over two lines, a raw string or a char literal
    before a comment, and Rust that does not lex;
  - an apostrophe in a YAML plain scalar;
  - a keyword and a name in a comment or a string, and a `use` list, in the
    index check;
  - a method report that lists the methods without an example;
  - a protocol-shape line overwritten or removed;
  - a benchmark run that fails with the base checked out.
- **Protocol shape history** (`scripts/check-shape-append-only.py`, on pull
  requests). `crates/bridge/protocol-shape.txt` only gains lines against the
  base revision: the script fails when a version line of the base is removed or
  changed. `crates/bridge/tests/protocol_version.rs` reads only the last line,
  so it catches a changed shape under the same version, not an old line
  overwritten.

## Doc examples

`scripts/check-example-coverage.py` reads nightly rustdoc's JSON for every
library crate under `crates/` (`seam-host` with its `vst3`, `cabi` and
`test-host` features, `seam-bridge` with `typescript`), walks it from the crate
root through public modules and re-exports as a user's `use` does, and fails
below 100 % in any crate. Every struct, enum, trait, function and macro a user
can reach has a Rust code block that is not marked `ignore`, `text` or
`compile_fail`. `#[doc(hidden)]` items are skipped. The examples themselves run
in `rust`: Test, as doc tests.

The same script also reports, and does not gate, how many public methods each
crate has with an example and without. A method is a public function in an
inherent `impl` of a reachable struct, enum or union, or a function of a
reachable trait; a trait impl's methods count at the trait. It lists those
without an example, because the Rust API Guidelines' level is an example on
every method whose summary does not tell the whole story, and the list is what a
docs pass reads.

## Property tests

Snapping, gesture nesting and undo coalescing are tested as properties over
generated inputs, not only as examples:

- a value snapped twice equals the value snapped once;
- any interleaving of begins and ends on several controls reaches the host as
  balanced pairs;
- any sequence of writes inside one gesture undoes in one step to the value
  before the gesture;
- with watches made and dropped among every kind of write and gesture, every
  watch left shows the gesture's value while one is open and the store
  otherwise.

They run proptest's default 256 cases on every pull request and 4,096 on a run
started by hand, through `PROPTEST_CASES`. A failing property test's
regression file is uploaded. Three properties are pinned differently, where
`PROPTEST_CASES` does not reach: one by examples (a no-op write keeps the redo,
NaN payloads included), the session race by a seeded loop of threads, and the
page's held queue by a seeded loop of 4,096 random write and call sequences
(`page/test/held.test.ts`).

Saved blocks are tested the same way without proptest.
`crates/host/tests/hostile.rs` generates random bytes and blocks shaped as saved
ones, every part hostile in turn (edge and out-of-range numbers, wrong types),
and cuts, flips and pads them. For each, a load never panics, is recognised or
not whatever state came before, changes nothing when it is not, and saves a
block that loads back to itself. Its seeded cases scale with `PROPTEST_CASES`
too. How deep a section may nest is pinned elsewhere.
`crates/session/tests/depth.rs` pins that a capture refuses what a load could
not read back. `crates/host/tests/state.rs` pins the deepest section through the
exporter's save and load.

## Fuzzing

`fuzz/` (outside the Cargo workspace; it needs a nightly compiler) holds four
`cargo-fuzz` targets, one per input boundary:

- `ump_packets`: splitting a stream, decoding each voice message, and encoding
  it back;
- `page_message`: the page's JSON;
- `saved_block`: a load of arbitrary bytes, then a save that must load back and
  save the same bytes;
- `cabi_state`: the same through the C interface's state calls.

The seeds in `fuzz/corpus/` are real messages, saved blocks and hostile
variants. The nightly lane starts from them plus the corpus earlier runs grew.
Run one locally with `cargo +nightly fuzz run <target>` from `fuzz/`. On Windows
the sanitiser's DLL (`clang_rt.asan_dynamic-x86_64.dll`, in the Visual Studio
compiler's `bin\Hostx64\x64`) must be on `PATH`.

In the nightly lane each target runs ten minutes, and a run passes with no
crash, no panic and no broken fixpoint; a crash is uploaded. Each leg also runs
the fuzz crate's own unit test (`cargo test --manifest-path fuzz/Cargo.toml
--lib`), which no other lane runs because the crate is outside the workspace.

## Coverage

`cargo llvm-cov` over the code Linux builds measured 93.5 % of lines on the day
the lane was added. Test files, benchmarks and the test hosts are left out. The
lane fails below 92 %, the measured figure less one point, rounded down to a
whole number. When coverage rises, raise the number in `ci.yml`'s
`COVERAGE_LINES` the same way; never lower it to land a change. The lane uploads
the report. It says nothing about Windows- and macOS-only code.

## Benchmarks

`scripts/run-benches.sh` runs the `iai` benchmarks of `seam-ump`, `seam-params`,
`seam-learn` and `seam-example-eq` under valgrind with iai-callgrind and prints
instruction counts, which do not vary with the machine's load. Given a base
revision it records the base's counts, then fails if any count at the current
revision is more than 2 % higher. A benchmark function is what is counted, so
each hands back the state its setup made, and freeing it falls outside the
count. A base without the benchmarks is run alone. On
a pull request they cover the EQ's block, a host write, learn's dispatch and UMP
decoding. Instruction counts say nothing about time or cache behaviour. The
runner's version must equal the `iai-callgrind` dependency's.
