# Architecture

Seam is a Rust framework for audio plugins. A plugin declares each parameter
once, and gets automation, undo, MIDI learn, saved state and a typed React page
from that one declaration. Hosts see the plugin through an
[exporter](docs/glossary.md#exporter): VST3 natively, and AU, AUv3, AAX and
hosting through a generic JUCE adapter driven by a C interface. A UI is one
React page in a WebView, talking to Rust through a transport the page does not
choose.

Three [tiers](docs/glossary.md#tier), dependencies pointing down only. The core
is `params`, `undo`, `session`, `learn` and `ump`. The projections are `bridge`
(the page) and `host` (plugin formats). Shells (a plugin crate, the JUCE
adapter) sit on top and may depend on both. Each rule below links the entry of
[the decision record](docs/decisions.md) that says why it exists,
where the record has one, and [docs/decision-index.md](docs/decision-index.md)
leads from each decision to its code. A word Seam coins links to [the
glossary](docs/glossary.md) where it first appears. To write a plugin before
reading how Seam works inside, start with
[docs/first-plugin.md](docs/first-plugin.md).

## Codemap

- `crates/params`: parameter declarations, typed
  [handles](docs/glossary.md#handle), the value
  [store](docs/glossary.md#store), write [origins](docs/glossary.md#origin),
  [gestures](docs/glossary.md#gesture), change counters.
- `crates/undo`: undo history over parameters, plain values and lists.
- `crates/session`: saved state: who contributes to it,
  [restore](docs/glossary.md#restore), late
  [participants](docs/glossary.md#participant).
- `crates/learn`: MIDI learn and the audio thread's dispatch table. Its
  capture and dispatch words are packed by `field.rs`, the controllers a plugin
  reserves are in `reservation.rs`, and the section it saves is
  `mappings/session.rs`.
- `crates/ump`: Universal MIDI Packets, the event list MIDI travels in.
- `crates/host`: the `Plugin` trait, the saved block (`SavedState`), and the
  exporters behind features: `vst3`, and `cabi` (the C interface and generated
  `include/seam.h`). The exporters are thin shells over one core, `engine`,
  which holds the plugin's life and builds with no feature. CLAP is planned,
  not built.
- `crates/bridge`: the page protocol and the TypeScript generated from it.
- `crates/testkit`: test tools: allocation gate, latches, expected assertions,
  drift check.
- `examples/gain`: the example gain, which wires parameters, undo, a saved
  value, MIDI learn and the VST3 export in one plugin;
  [docs/first-plugin.md](docs/first-plugin.md) walks through it.
- `examples/eq`: the example EQ, the reference for performance and parity, as a
  VST3 module or a static library for the adapter.
- `page`: React hooks for the page, over a pluggable transport. Its client
  holds writes made before the native side answers in `src/held.ts`, and limits
  a position to 0 to 1, which the bridge's decoding refuses outside.
- `adapters/juce`: the generic JUCE adapter: any Seam plugin, through `seam.h`,
  as a JUCE plugin.
- `xtask`: `cargo xtask gen` writes every generated file; each has a drift gate.
- `bench`: the comparison against plain JUCE.

## From a host's call to your block

Read these in order to change what happens to a block.

1. A plugin implements `Plugin` (`crates/host/src/plugin.rs`): its `INFO`, its
   `PARAMS`, a saved state, and `activate`, which builds the `Processor`.
   `examples/eq/src/lib.rs` is a whole one, `examples/eq/src/params.rs` its
   parameters; `examples/gain/src/lib.rs` adds undo, a saved value and learn.
2. An exporter (`vst3/component.rs`, `cabi/instance.rs`) turns the host's calls
   into calls on `Engine` (`crates/host/src/engine/mod.rs`), which holds the
   instance's life: [activation](docs/glossary.md#activation), the
   [tick](docs/glossary.md#tick), what the host is told, teardown. The table
   below maps each entry point to its `Engine` method.
3. A [block](docs/glossary.md#block) is `Engine::process` (`engine/process.rs`).
   On the audio thread it hands the exporter a `Points` over the engine's
   `Automation` to fill (step 4), applies it to the parameter set, and flushes
   denormals. Then `run_held` takes the processor out of its `AudioSlot`, and
   calls `Processor::process` with a `Block` (`crates/host/src/audio.rs`) in
   [pieces](docs/glossary.md#piece) of at most the activation's `max_frames`.
   `Channels::run` (`boundary/channels.rs`) builds each piece's channels in
   place. Channel `c` is the host's output buffer `c` when that overlaps no
   earlier channel's, and a scratch buffer otherwise. The host's input is
   copied into it unless it is already there. A scratch buffer also stands in
   for a channel the host gave none. The processor changes the audio in place.
4. The host's automation is `engine/automation.rs`. A VST3 host sends queues of
   points, each at a sample offset (`Point`); the exporter hands the engine one
   queue at a time through `Points::queue`. The engine has
   [room](docs/glossary.md#room) for 8 points per parameter on average, fixed
   when it is built. A parameter's first queue wins: a second queue for it is
   passed over unread. The engine reads the winning queue's last point first and
   applies it before the block, so the processor sees one value per parameter
   per block. Earlier points are carried while the room lasts and counted as
   dropped after that. A C-interface host sends no queues (`HostLink::QUEUES` is
   false), so the engine has no room and a write lands at once through
   `Engine::host_write`.
5. A parameter reaches the block through one atomic cell per parameter in
   `ParamSet` (`crates/params/src/set.rs`). A host write is a 0 to 1 position
   that `AudioParams::host_write` turns into a legal value
   (`ParamDecl::host_value`) and stores, with its origin, in one atomic word
   (`crates/params/src/set/value_cell.rs`). The processor reads
   `ParamSet::value` and compares `ParamSet::changes` at the top of its block,
   and recomputes only what moved (`retune` in the example EQ). A reader that
   needs a value, its count and its origin together asks `ParamSet::read`,
   which returns a `Reading`.
6. A write from the page or an editor goes the other way: `UiParams` stores into
   the same cell, and at the next tick the `HostEventDrain` (a
   [drain](docs/glossary.md#drain)) tells the host.

`HostLink` (`engine/mod.rs`) is how a shell tells the engine what its host is
like. `QUEUES` says whether the host sends automation as queues with each block:
VST3 does, the C interface does not. `ACTIVATION_TELLS_LATENCY` says whether the
host reads the latency as an activation returns. The engine is generic over it,
so one engine serves both shells.

| Host's call | VST3 (`vst3/component.rs`; the controller half in `vst3/component/controller.rs`) | C interface (`cabi/mod.rs`) | `Engine` method |
| --- | --- | --- | --- |
| One block, with its automation | `process` | `process` | `process` |
| A write outside a block | `setParamNormalized` | `set` | `controller_write` (VST3), `host_write` (C) |
| Start or stop processing | `setProcessing` | none | `set_processing` |
| Reset the processor | a stop in `setProcessing` | `reset` | `request_reset` |
| Activate or deactivate | `setActive` | `activate`, `deactivate` | `activate`, `deactivate` |
| A tick | the exporter's timer | `idle` | `tick`, `idle` |

To prove a change, run the tests that reach it. `engine/tests.rs` tests each of
the engine's rules through both exporters with one body.
`examples/eq/tests/matrix.rs` and `overflow.rs` drive the example EQ through
both exporters across block sizes, rates and layouts, and with automation that
overflows the room. The allocation gates (`alloc_gate.rs` in `crates/host/tests`
and `examples/eq/tests`) fail on an allocation in the block. The loom models
check the hand-offs (`scripts/run-loom.py`): the parameter cell, the host event
drain, learn's staged value and its capture staging, and in
`boundary/loom_models.rs` the main-thread state and the processor slot.
[docs/testing.md](docs/testing.md) says what each proves and how to run it, and
which audio-path modules deny the lints that refuse indexing, `unwrap` and
unchecked arithmetic.

The engine's items are `pub(crate)`, so `cargo doc` hides them. Read their docs
with `cargo doc -p seam-host --features vst3,cabi,test-host
--document-private-items --open`.

## Invariants

- The core crates and `bridge` never depend on a window, WebView, UI toolkit or
  plugin format, with any features, on any platform.
  `scripts/check-core-deps.py` checks it in CI ([where the WebView pieces
  live][webview]).
- `undo` and `session` never depend on each other. Both take the same restore
  clock from the parameter set that owns it ([who makes the clock][clock]). A
  load moves the clock with every parameter it writes, under the lock a replay
  step checks it under, so no step for a change finished before a load replays
  over it ([a load and a replay][load]). A user's gesture still open when the
  load lands, a lone write included, keeps its value from before the gesture,
  and undoing it writes that value.
- `unsafe` is denied workspace-wide. Only `host`'s `vst3` and `cabi` modules
  and the `boundary`, `engine` and `faults` modules they share allow it, with a
  `// SAFETY:` comment on every block.
- A panic never unwinds into a host. An exporter catches it at the entry point,
  counts it as a fault, and a processor that panicked plays silence until the
  next activation. The first fault of each kind in an instance is reported as a
  line of text, never on the audio thread
  ([`faults.rs`](crates/host/src/faults.rs) says when, and on which thread;
  [a panic in a host's call][panic], [making a fault visible][faults]).
- The audio path never allocates, locks or makes a system call, except while a
  panic unwinds (`faults.rs` says why). Allocation gates, and in a debug build
  the exporters' audio-thread mark, check what the tests reach;
  [testing.md](docs/testing.md#2-the-audio-path-does-not-allocate) says what
  each sees and what is left to review ([checking the audio-thread
  rule][mark]).
- A saved block names two versions, each its own: `seamStateVersion` for the
  frame Seam gives it, and `pluginStateVersion` for the plugin's schema. A save
  writes the higher of the block's and the build's for each, and keeps the keys
  it does not read, so a block a newer build wrote reaches it again whole
  ([the saved block's two versions][versions], [upgrading an older
  block][migration]).
- Generated files (the page's protocol types, its test parameter map, `seam.h`,
  the example EQ's JUCE settings) are committed and compared byte for byte in
  CI ([what runs the generator][generator], [keeping the header true][header]).
- Absent: no WebView glue exists yet, so nothing in the repository opens a page
  in a plugin window, and no exporter delivers MIDI events yet ([where the
  WebView pieces live][webview], [the VST3 exporter's shape][vst3]).

## Threads

- [Main](docs/glossary.md#main-thread): owns the plugin, builds the `Processor`
  at activation, takes it back at deactivation. A tick calls `Plugin::idle`
  about 60 times a second and then tells the host of the plugin's
  [edits](docs/glossary.md#edit). A session restore applies to its participants
  on the plugin's idle, when that ticks its `SessionUi`; `Plugin::idle` says
  what else it ticks, and in what order. The JUCE adapter ticks through JUCE on
  every platform. The VST3 exporter runs its own timer on Windows and macOS, and
  refuses to build for any other platform. There nothing would tick it: no edit
  would reach the host, and a load would never apply session sections to their
  participants ([what ticks a plugin][tick], [when a restore
  applies][applies]).
- Audio: owns the processor and never waits. A host write may arrive here, and
  so may the C interface's `get`, which the JUCE adapter's parameter reads call
  there ([the C interface][cabi]). The audio thread reads parameter values
  straight from the set's atomic cells.
- Any but audio: the host's save and load, and its text and query calls. Save
  and load go through `SavedState`. Text and query calls read the declarations,
  the parameter store, and the latency, tail and layout the main thread last
  read. None touches the plugin ([whether a plugin must be Send][send]).
- Which thread is main: VST3 builds the plugin at `initialize`, on the thread
  that calls it, which stays its main thread. The C interface keeps a plugin
  that is not `Send` on the thread that called `create`, and lends one that is
  `Send` to each main-thread call in turn, on any thread (a
  [loan](docs/glossary.md#loan)). The state is `MainThread` (`boundary.rs`),
  bound to one thread or lent call by call. The rule in full is on `Engine`
  (`engine/mod.rs`) for Seam's code, on `Plugin` (`plugin.rs`) for plugin
  authors, and on `SeamPlugin` in `seam.h` for C users ([whether a plugin must
  be Send][send], [the VST3 exporter's shape][vst3], [the C interface][cabi]).

## Decided, not built

- Dependency edges: `host` on `learn`, so an exporter refuses a plugin that uses
  learn without declaring MIDI input; and a shell crate, `seam-webview`, on
  `bridge` and wry.
- The page path: wry inside a plugin window, Tauri for the standalone.
  `seam-webview` holds the wry glue (the IPC handler, the custom-scheme asset
  server, opening the page in a window). A release build serves the page from
  bytes in the binary over that scheme; a debug build loads a dev server when
  `SEAM_PAGE_DEV_URL` is set, which release builds never read. Open: that macOS
  WebKit inside a host process loads plain `http://127.0.0.1` is not verified.
- MIDI-CI: a VST3 host speaks it for the plugin, which states what it supports
  through VST3's interfaces. Everywhere else a core crate, `seam-ci`, answers
  with UMP in and out, SysEx7 included, through the C interface and CLAP. Open:
  the SysEx manufacturer ID its identity needs.
- Learn in VST3: hidden proxy parameters in a reserved id range carry
  controllers. They form a grid of 16 channels by 130 (the 128 control changes,
  channel pressure, pitch bend), mapped through `IMidiMapping`. A pool of them
  for MIDI 2.0 assignable controllers is bound through `IMidiMapping2`. The
  `vst3` crate's `Midi2Controller` is four bytes where the SDK's is two, so the
  exporter must define its own.
- CLAP: an exporter over the same engine, ticked through the host's
  timer-support extension. Open: a CLAP host may lack timer support.
- Learn's snapshot fallback: should a later `triple_buffer` break what the
  snapshot rests on, the fallback is a hand-written table with a reader count
  and `SeqCst` fences. Its `unsafe` is under the boundary's rules, with a miri
  job.

[applies]: docs/decisions.md#d13-when-a-session-restore-applies
[cabi]: docs/decisions.md#d62-the-c-interface-for-the-juce-adapter
[clock]: docs/decisions.md#d52-who-makes-the-restoreclock
[faults]: docs/decisions.md#d75-making-a-caught-fault-visible
[generator]: docs/decisions.md#d27-what-runs-the-generator
[header]: docs/decisions.md#d63-keeping-the-c-header-true
[load]: docs/decisions.md#d39-a-load-between-a-replays-check-and-its-write
[mark]: docs/decisions.md#d67-checking-the-audio-thread-locking-rule
[migration]: docs/decisions.md#d80-upgrading-an-older-saved-block
[panic]: docs/decisions.md#d60-a-panic-inside-a-hosts-call
[send]: docs/decisions.md#d51-whether-a-plugin-must-be-send
[tick]: docs/decisions.md#d53-what-ticks-a-plugin-with-no-editor-open
[versions]: docs/decisions.md#d84-the-saved-blocks-two-versions
[vst3]: docs/decisions.md#d59-the-shape-of-the-vst3-exporter
[webview]: docs/decisions.md#d16-where-the-webview-pieces-live
