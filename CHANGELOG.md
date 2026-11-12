# Changelog

All notable changes to Seam are recorded here. The format follows [Keep a
Changelog](https://keepachangelog.com/en/1.1.0/), and versions follow [Semantic
Versioning](https://semver.org/spec/v2.0.0.html): before 1.0, a minor version
may break the API.

## [Unreleased]

Nothing is released yet. Not built: the CLAP exporter, the native side that
answers the page, the glue that opens the page in a plugin window, and MIDI
delivered to a plugin through an exporter.

### Added

- `seam-params`: parameters declared once and checked by a builder, with one
  atomic store per parameter in real units. Every write snaps to the parameter's
  kind, never stores a non-finite value, and names its origin, which alone
  decides whether the host is told, a gesture opens, undo records and surfaces
  show it. Gestures nest and count, a program write is an edit or a sync, and
  change counters, watches and the host event drain carry changes. A parameter's
  value and origin share one atomic word, and `ParamSet::read` and
  `Value::read` return a `Reading`: the count, the value and the origin taken
  together, as `AudioParams` does on the audio thread. Plain values and lists
  hold state that is not a parameter.
- `seam-params`: typed parameters. A `Float`, a `Bool` or a `Choice<E>`
  carries its store and reads with `get`, on any thread, as an `f32`, a `bool`
  or the plugin's own enum. `ParamSet::bind` makes one by key and checks it
  against its declaration once, refusing a missing key, another kind or a
  choice whose labels are not the enum's. `UiParams::write` takes a typed
  parameter with a value of its type, or a `ParamId` with an `f32`. `Changes`
  asks once per block whether any of the parameters it is given moved.
- `seam-params`: parameters declared where they are held. A struct
  implements `Params`, whose `declare` adds each parameter through a
  `Declarer`; a typed declaration, `FloatDecl`, `BoolDecl` or `ChoiceDecl<E>`,
  hands back its kind of parameter, so a field of another kind does not
  compile. `checked`, a `const fn`, refuses a bad key, group, choice labels,
  range or default while compiling. `ParamSet::declare` builds the set and
  returns the first declaration it refused, such as a key two parameters
  share. A typed declaration refuses a default outside its range, where a
  `ParamDecl` snaps it. `Declarer::extend` declares a plain table.
- `seam-params`: `#[derive(Params)]`, under the `derive` feature, writes a
  struct's `declare` from a `#[param(...)]` mark on each field: the
  declaration's arguments, then `name = value` options and lone-word flags,
  with the kind taken from the field's type through `Declared`. A field with
  no mark declares a struct of parameters. Each declaration is checked while
  compiling, and so is a key two fields of the struct share: `cargo check`
  reports it on the mark. The example gain derives its parameters.
- `seam-params`: `#[derive(Choices)]`, under the same feature, writes a
  choice's enum: its labels and both mappings, in the order of the variants.
  A label is the variant's name unless a `#[choice("...")]` mark gives one. A
  written discriminant must be the variant's position, and a label no choice
  could have is refused: `cargo check` reports either on the author's line.
  The example gain's `Mode` derives it.
- `seam-undo`: an undo history that polls once per tick. One gesture is one
  step, transactions group steps, and host automation, restores and learned
  writes never record. A restore clears the history, and the history is bounded
  by the bytes its steps keep alive. `undo`, `redo` and `jump` return false
  when there is nothing to replay, or when a restore stops the walk, which
  clears the history. Enrolling a value or a list does not tick, unlike
  enrolling a parameter, so another value's writes on either side of it
  coalesce into one step.
- `seam-session`: saved state beyond parameters, one keyed section per
  participant. A restore stores every section at once and applies on a later
  tick; a participant whose apply fails or panics keeps its saved section, and
  sections no participant claims are saved back. A participant held up by its
  own borrow is captured at the next tick, and one whose apply panicked is
  faulted until an apply succeeds. A capture nested too deep, or too large for
  the saved block to load back, is refused and the previous one kept, whether
  the participant registered before the saved state was made or after.
- `seam-learn`: MIDI learn over MIDI 1.0 and 2.0 packets. Arm a target, move a
  controller, and the mapping drives the target from the audio thread through a
  published snapshot that never waits, locks or frees. Notes, NRPNs, 14-bit
  controller pairs, curves, reservations and a saved `midiLearn` section. Half
  of a 14-bit pair waits at least a UI tick and a whole audio block for its
  other half. `Mapping` and `Config` are `non_exhaustive`, built with
  `Mapping::new`, `with_curve` and `as_coarse`.
- `seam-ump`: Universal MIDI Packets: a fixed-capacity event list per block, and
  the channel voice messages read from it and written back.
- `seam-host`: the `Plugin` and `Processor` traits and `PluginInfo`, and
  `SavedState`, the saved block as JSON, which names Seam's frame version and
  the plugin's own schema version apart. A load keeps unknown parameter ids and
  top-level keys, migrates a block of an older plugin version or fails with the
  migration's reason, and refuses what is past fixed limits, a block that would
  save back past the size limit included; a setup past the block size limit is
  built for the limit. A plugin that is `Send` is lent to each main-thread call
  in turn, and a call that meets another thread's loan waits a bounded time
  before it is refused.
- `seam-host`: Seam builds each instance's parts. `Plugin::Params` names the
  struct that declares the parameters a host is shown; an exporter declares it,
  builds the set, the session and the saved state over it, and hands
  `Plugin::new` the parameters and the `Parts`. A declaration refused fails the
  instance, with the reason in the fault report. `Parts::undo` and
  `Parts::midi_learn` turn undo and learn on, and return no `Result`: a
  mistake in the plugin's source, such as either turned on twice, panics with
  what to change, which refuses the instance. While a plugin keeps its parts,
  Seam ticks them and the session before each `Plugin::idle`, which holds only
  the plugin's own work; a plugin never ticks them. A plugin declares its
  saved-state migration as `Plugin::SCHEMA_VERSION` and `Plugin::migrate`.
  `Parts::for_plugin` builds a plugin's parts with no host, for tests, which
  tick them through a `Driver`.
- `seam-host`: a plain value saved with the session in one call.
  `Parts::saved_value` makes a value of any type serde can write and read,
  starting at a default, and keeps it saved under a key, as JSON; a project
  saved without the key loads the same default. A
  change is captured at the next tick, or at once by `Parts::sync`, and a load
  is captured once, by its apply.
- The VST3 exporter (`export_vst3!`), for Windows and macOS, with its own
  main-thread timer.
- The C interface (`export_cabi!`) and its generated header, `include/seam.h`,
  which asserts every struct's layout as Rust laid it out.
- One exporter core under both. The core catches every panic at the boundary,
  counts and reports faults, and flushes denormals around each block. It stages
  each piece of a block's channels in place, and a model of every way a host may
  hand its buffers checks that. Test hosts drive a plugin through either
  exporter, and answer a refused call with what the host was told.
- `seam-bridge`: the page protocol, versioned, with its TypeScript generated
  from the Rust types and a check that the version moves when a message's shape
  does. Decoding refuses a position outside 0 to 1, from either side.
- `page`: React hooks (`useParam`, `useEvent`, `useCommand`) over a pluggable
  transport. A held queue keeps the writes made before the native side answers
  and sends them in the order the page made them. A native message of the wrong
  shape fails the client visibly.
- `adapters/juce`: a generic JUCE adapter that builds any Seam plugin as a JUCE
  plugin, through `seam.h` and `seam_add_juce_plugin`, for the formats JUCE
  builds. Checked as VST3 on Windows; CI builds VST3 and AU on macOS.
- `examples/gain`: a gain in dB with a bypass, a mode of three names, an
  undoable label saved with the plugin, and a gain MIDI learn can drive, as a
  VST3 module. [docs/first-plugin.md](docs/first-plugin.md) walks through it.
- `examples/eq`: an EQ of six biquad bands and an output gain, as a VST3 module
  or a static library for the adapter.
- `xtask`: `cargo xtask gen` writes every generated file, and CI fails when one
  drifts. `cargo xtask wrap` holds comments and docs to 80 columns.
- `bench`: the comparison of the example EQ against the same EQ in plain JUCE,
  with its reports in `bench/reports`.
- Guarantees the tests keep: the audio path never allocates (allocation gates),
  the lock-free hand-offs pass `loom` models, `unsafe` lives only in the
  exporters' boundary modules, the core crates build with no window, WebView or
  plugin-format dependency, and fuzz targets cover the UMP reader, the page
  messages, the saved block and the C interface's state.
