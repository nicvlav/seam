# Your first plugin

This guide walks through [`examples/gain`](../examples/gain/src/lib.rs), a
stereo gain that uses the parts of Seam most plugins need: parameters, saved
state, undo, a value that is not a parameter, MIDI learn, and the VST3 export.
It leaves two things out: it has no editor or page, so a host shows its
generic controls, and nothing smooths its gain. Read it beside the source,
which fits one sitting. The guide follows the order in which you would write
the plugin, and links the item that owns each rule rather than repeating it.
Every block of Rust and TOML below is quoted from the example or its tests,
and its `guide` test fails when the two drift apart.

The plugin has four things to set:

- a gain in dB, which MIDI learn can drive;
- a bypass;
- a mode with three names: Clean, Warm and Hot;
- a label, saved with the project and undoable.

The example EQ (`examples/eq`) is the other example: parameters and saved
state only, the reference for performance and for parity with plain JUCE.

## The crate

A plugin is one library crate built two ways: a `cdylib`, the module a host
loads, and an `rlib`, which its tests drive in-process. The parts of the
example's [manifest](../examples/gain/Cargo.toml) that a plugin needs:

```toml
[lib]
# The cdylib is the VST3 module; the rlib lets tests drive it in-process.
crate-type = ["cdylib", "rlib"]

[features]
default = ["vst3"]
# The VST3 module's entry points, through Seam's exporter.
vst3 = ["seam-host/vst3"]

[dependencies]
seam-host.workspace = true
seam-learn.workspace = true
seam-params = { workspace = true, features = ["derive"] }
seam-undo.workspace = true

[dev-dependencies]
seam-host = { workspace = true, features = ["vst3", "test-host"] }
seam-ump.workspace = true
```

`seam-host` builds no exporter by default, so the crate's own `vst3` feature
turns the VST3 one on, and the tests add the test host. `seam-params`'s
`derive` feature gives `#[derive(Params)]`. Each
`workspace = true` takes its value from the repository's root manifest;
[your own crate](#your-own-crate) says what replaces it outside this
repository.

## 1. Declare the parameters

A parameter is declared once, where the plugin holds it. The gain's sit in a
struct of their own, which derives [`Params`](../crates/params/src/lib.rs):
the mark on each field declares that parameter, and the derive writes the
function that declares them all and keeps what each is handed:

```rust
#[derive(Debug, Clone, Params)]
pub struct GainParams {
    #[param(
        "gain", "Gain", -60.0..=12.0, 0.0,
        step = 0.1, unit = "dB",
    )]
    gain: Float,
    #[param("mode", "Mode", Mode::Clean)]
    mode: Choice<Mode>,
    #[param("bypass", "Bypass", false, bypass)]
    bypass: Bool,
}
```

Each mark is a key, a display name, a range for a float, and a default in
real units, then its options. Everything else follows from it: the host's id
(a hash of the key), its range and text, the save, and the page's typed hook.

- The field's type is the parameter's kind, and says which declaration the
  mark writes: a [`FloatDecl`](../crates/params/src/declare.rs) for a `Float`,
  a `BoolDecl` for a `Bool` and a `ChoiceDecl` for a `Choice<Mode>`. The
  mark's leading items are that declaration's `new`, `step = 0.1` is its
  `with_step(0.1)`, and the lone word `bypass` its `as_bypass()`. So a
  float's arguments on a `Bool` field do not compile.
- A float's range reads as JUCE's `NormalisableRange (start, end, interval)`:
  ends included, and `step`, where 0, the default, means no steps.
  `skew_centre` puts a value at the middle of the control's travel, as
  `setSkewForCentre` does. So the gain runs from -60 dB to 12 dB in steps of
  0.1 dB, linearly. A default outside the range is refused, not moved into it.
- Each declaration is checked while compiling: a key or a range no parameter
  could have, a default outside its range, and a key another field has fail
  `cargo check`, on the mark. rustfmt leaves a mark as it is typed, so a long
  one is wrapped by hand, as the gain's is: the arguments, then the options.
- The store keeps real units (dB, not 0 to 1). A host speaks in 0 to 1
  positions, and Seam converts both ways at its boundary.
- A choice's names are its labels, and the host sees them as the parameter's
  text. The mode's come from the enum that names its values, so the two cannot
  drift apart.
- `bypass` tells the exporter this parameter is the host's bypass. The
  processor still implements it: the host only learns which parameter it is.
- Hosts list the parameters in the order of the fields.
- The key is the parameter's identity for good. The saved block stores the
  value under it and the host's id is its hash, so a key never changes once
  the plugin ships, though the display name may. Adding a parameter later is
  safe: a block saved before it loads with the new parameter at its default
  ([`SavedState::load`](../crates/host/src/state.rs)).

The derive writes what you would write by hand: a `declare` that adds each
field's declaration, `p.add(FloatDecl::new(...).with_step(0.1))`, and returns
the struct. Write it yourself when the parameters are not one per field. The
example EQ's [`EqParams`](../examples/eq/src/params.rs) does: its six bands
come from a table, a band's shape is not a parameter, and a high-pass has no
gain.

`Mode` names the choice's values, in the order of its labels:

```rust
impl Choices for Mode {
    const LABELS: &'static [&'static str] = &["Clean", "Warm", "Hot"];
```

`from_index` must give a mode for any index, since the audio thread calls it;
`Hot` takes every index past the last.

Each field is a [typed parameter](../crates/params/src/param.rs) that carries
its store: `get` returns the gain in dB as an `f32`, the mode as a `Mode` and
the bypass as a `bool`, on any thread, with no lookup or threshold. A clone is
the same parameter, sharing the store. Seam calls `declare` when it builds an
instance, and a declaration it refuses, such as a key two parameters share,
fails that build with the reason, in any test.

## 2. Write the processor

[`Processor`](../crates/host/src/plugin.rs) is the audio thread's half, your
`processBlock`. The host calls `process` with a
[`Block`](../crates/host/src/audio.rs): the channels, edited in place, and the
block's MIDI events.

`GainProcessor` keeps the parameters' values in plain fields and reads them
again only when one moved. It holds a clone of the parameters and a
[`Changes`](../crates/params/src/changes.rs), which keeps a stamp of the
parameters it was last given and their change counts, and answers whether any
of them took a write since it last asked:

```rust
fn retune(&mut self) {
    let GainParams { gain, mode, bypass } = &self.params;
    if !self.changes.moved([gain, mode, bypass]) {
        return;
    }
    self.gain = 10.0_f32.powf(gain.get() / 20.0);
    self.mode = mode.get();
    self.bypassed = bypass.get();
}
```

The parameters carry their stores, so the processor needs no handle to the set
to read them. Reading takes atomics only, and the audio thread never
allocates, locks or waits ([the invariants](../ARCHITECTURE.md#invariants)).
The loop itself is plain DSP.

Open: Seam has no smoothing yet. The gain is read once per block, at its
start, so a change steps there, and a large one can click.

## 3. The plugin and its parts

[`Plugin`](../crates/host/src/plugin.rs) is the main thread's half. Its
constants name the plugin (`INFO`), and its `Params` is the struct above.
Seam declares it, builds the parameter set, the session and the saved state
over it, and hands the parameters and those
[`Parts`](../crates/host/src/parts.rs) to `new`. `new` keeps what the plugin
uses; `activate` builds a fresh processor each time the host activates it, so
nothing the audio thread touches is allocated in a block:

```rust
fn activate(&mut self, _setup: &ProcessSetup) -> GainProcessor {
    GainProcessor::new(self.params.clone(), self.parts.learn_audio())
}
```

The host saves and loads through the saved state in the parts, on threads of
its own, without touching the plugin. The saved block is JSON: Seam's version
and the plugin's own, the parameters' values, and one keyed section per
session participant. A later version of the plugin that renames or rescales a
parameter raises its `SCHEMA_VERSION` and upgrades an older block in its
`migrate`, as it loads.

The parts hold the main thread's handles, so a plugin that keeps them is not
`Send`, and runs on the thread that created it. The example EQ keeps only its
parameters and drops its parts, so it stays `Send`.

## 4. Undo

Undo is one call in `new`, which turns it on with every parameter enrolled:

```rust
let history = parts.undo(Bound::default()).expect("no other history");
```

The `History` it returns records changes to what is enrolled in it. Only
writes through the parts' [`UiParams`](../crates/params/src/ui.rs) handle,
the one an editor writes parameters through, record, as a user's: a host's
automation, a load and a learned controller never do, so undo never fights
the host. A drag is one [gesture](glossary.md#gesture) and so one undo step,
and so is each lone `write`, a click say, which is a gesture of one value. The
history records once per tick, but an undo first records what changed since
the last one, so it needs no tick before it. From the example's tests:

```rust
let gain = plugin.ui().set().id(GAIN).expect("declared");

// Each lone write is a gesture of its own, so each is a step. Undo
// records what changed since the last tick before it undoes.
assert!(plugin.ui().write(gain, -6.0));
assert!(plugin.ui().write(gain, -12.0));
assert_eq!(plugin.history().undo(), Ok(true));
assert_eq!(plugin.ui().value(gain), -6.0);
```

## 5. A value that is not a parameter

The label is state the host knows nothing about. A
[`Value<String>`](../crates/params/src/value.rs) holds it, and three lines in
`new` give it a life:

```rust
let label = Value::new(DEFAULT_LABEL.to_owned(), parts.set());
history.enroll(&label, "Rename").expect("a new value");
parts
    .save(LABEL_KEY, &label, DEFAULT_LABEL.to_owned())
    .expect("a free key");
```

Enrolling it makes its changes undo steps: the sets between two ticks are one
step, the latest winning, where two lone writes of a parameter are two. A
[`List`](../crates/params/src/list.rs)'s verbs are one step each instead. The
same test goes on with the label:

```rust
// The label's sets between two ticks are one step, the latest winning.
plugin.set_label("Vocals");
plugin.set_label("Lead vocals");
assert_eq!(plugin.history().undo(), Ok(true));
assert_eq!(plugin.label(), "Untitled");
```

[`Parts::save`](../crates/host/src/parts.rs) saves it with the project under a
stable key, `"label"`, as JSON through serde, so any value serde can write and
read is saved the same way. A project saved before the label existed loads the
default. A change, an undo's included, reaches the saved block at the next
tick; that method says why and what it costs. A load writes the parameters at
once, before it returns, but the label at the next tick.

## 6. MIDI learn

Learn is one more call in `new`:

```rust
parts
    .midi_learn(Config::default())
    .expect("learn's key is free");
```

It registers learn's own participant, so its mappings save under their own
key; the call fails only when that key is taken. `activate` above hands each
fresh processor learn's audio side, a
[`LearnAudio`](../crates/learn/src/audio.rs), from `parts.learn_audio()`. The
processor calls `learn.process(block.events)` at the top of `process`.
Learned writes then land before the parameters are read, in the block the
controller arrived in.

To learn, the plugin calls `Learn::arm`, through `parts.learn()`, on the
gain's id, the parameter's `id()`, with `Takes::CONTROLLER`. The next
controller the processor sees is captured on the audio thread, and the next
tick commits it.

The plugin declares `midi_input: true` in its `INFO`; learn refuses a plugin
that does not. Open: no exporter delivers MIDI events yet, so learn is
exercised by driving the processor directly, as the tests below do.

## 7. The idle tick

About 60 times a second on the main thread, Seam ticks the parts the plugin
keeps: [`Driver::tick`](../crates/host/src/parts.rs) runs learn, the session,
the history and the saved values in the one order that works, and owns the
reasons for it. The plugin writes nothing for it, and the gain writes no
`idle`. `Plugin::idle` is for a plugin's own main-thread work, and runs right
after Seam's tick, so it sees a restore applied and a learned mapping
committed.

## 8. Test it through a host

`seam_host::vst3::TestHost` drives the plugin through the VST3 exporter's own
interfaces, as a host would.
[`examples/gain/tests/gain.rs`](../examples/gain/tests/gain.rs) uses it for
what a host can reach: it sets up, activates and starts the host, each call
answering `Ok(())` or the plugin's refusal, then calls `process_in_place` with
changes given by host id, in the host's 0 to 1 (`normalize` converts dB first),
reads the modes' names through `parameter` and `text`, and carries the saved
block through `save` and `load`.

Learn and undo are driven on the plugin itself, since a host cannot reach them.
The test builds it as an exporter does, keeping a
[`Driver`](../crates/host/src/parts.rs) to tick its parts as the exporter's
engine would, and the saved state a host would save and load through:

```rust
fn built() -> (Gain, Driver, Arc<SavedState>) {
    let (params, parts) = Parts::for_plugin::<Gain>().expect("valid declarations");
    let driver = Driver::of(&parts);
    let saved = Arc::clone(parts.saved());
    (Gain::new(params, parts), driver, saved)
}
```

Then it arms the gain, feeds a controller through the processor, ticks the
driver, and saves and reloads to check the mapping survives. Run the example's
tests, this guide's check among them, with:

```sh
cargo test -p seam-example-gain
```

[docs/testing.md](testing.md) says what each CI lane proves.

## 9. Build and bundle the module

`export_vst3!(Gain)` at the top of the crate's `lib.rs` turns the `cdylib` into
a VST3 module, under the crate's own `vst3` feature from the manifest above.

```sh
cargo build -p seam-example-gain --release
```

A VST3 bundle is a folder. On Windows the library goes in as the module itself,
under the architecture's folder, as CI lays out the EQ's bundle:

```sh
bundle="bundle/Seam Example Gain.vst3"
mkdir -p "$bundle/Contents/x86_64-win"
cp target/release/seam_example_gain.dll \
  "$bundle/Contents/x86_64-win/Seam Example Gain.vst3"
```

Copy the bundle to `C:\Program Files\Common Files\VST3` and rescan in the host.
The plugin has no editor, so the host shows its own generic controls for the
three parameters. [pluginval](https://github.com/Tracktion/pluginval) checks a
bundle without a host. CI runs version 1.0.4 on the EQ's bundle with these
options; for the gain's:

```sh
pluginval --strictness-level 5 --validate-in-process --skip-gui-tests \
  "bundle/Seam Example Gain.vst3"
```

Open: CI lays out and validates the EQ's Windows bundle only, not the gain's.
The macOS bundle is not built yet.

## Your own crate

Seam's crates are not published yet, so a plugin outside this repository takes
them from git or from a checkout. Copy `examples/gain` to a folder of its own,
delete `tests/guide.rs`, which checks this guide against the example, and
change the manifest:

- In `[package]`, give each `.workspace = true` a value of your own: edition
  2024 and a `rust-version` of 1.99 or later, as Seam's.
- Point each Seam dependency, in `[dev-dependencies]` too, at where it lives:
  `seam-host = { git = "https://github.com/nicvlav/seam", rev = "..." }`,
  or `seam-host = { path = "../seam/crates/host" }` for a checkout. Pin one
  `rev` for every Seam crate, since before 1.0 a minor version may break the
  API. A dependency with `features`, `seam-params` and the dev-dependency on
  `seam-host`, keeps them beside its source.
- Copy the root manifest's `[workspace.lints.rust]` and
  `[workspace.lints.clippy]` tables in as `[lints.rust]` and `[lints.clippy]`,
  or drop the `[lints]` table, and copy its `[profile.release]` for the same
  release build.

## Where to read on

- [ARCHITECTURE.md](../ARCHITECTURE.md) follows a host's call to your block.
- Each crate's overview has its invariants and one worked example:
  [params](../crates/params/src/lib.rs), [undo](../crates/undo/src/lib.rs),
  [session](../crates/session/src/lib.rs), [learn](../crates/learn/src/lib.rs)
  and [host](../crates/host/src/lib.rs).
- The [glossary](glossary.md) defines the words Seam uses in a sense of its own.
- The example EQ shows a larger processor, with the allocation gates and the
  parity tests that prove it.
- [docs/style.md](style.md) is how code and docs are written here.
