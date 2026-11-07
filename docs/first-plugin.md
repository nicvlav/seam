# Your first plugin

This guide walks through [`examples/gain`](../examples/gain/src/lib.rs), a
stereo gain that uses every part of Seam a real plugin uses: parameters, saved
state, undo, a value that is not a parameter, MIDI learn, and the VST3 export.
Read it beside the source, which fits one sitting. The guide follows the order
in which you would write the plugin, and links the item that owns each rule
rather than repeating it. Every block of Rust and TOML below is quoted from the
example, and its `guide` test fails when the two drift apart.

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
seam-params.workspace = true
seam-session.workspace = true
seam-undo.workspace = true

[dev-dependencies]
seam-host = { workspace = true, features = ["vst3", "test-host"] }
seam-ump.workspace = true
```

`seam-host` builds no exporter by default, so the crate's own `vst3` feature
turns the VST3 one on, and the tests add the test host. The VST3 module
refuses to build on Linux, so a plugin crate added to this workspace also
joins the `--exclude` lists of the CI steps that build without a host: the
core-alone build and the cross check in `ci.yml`, and the miri run in
`nightly.yml`.

## 1. Declare the parameters

A parameter is declared once, as a [`ParamDecl`](../crates/params/src/decl.rs):
a key, a display name, a [`Kind`](../crates/params/src/decl.rs) and a default
in real units. Everything else follows from the declaration: the host's id (a
hash of the key), its range and text, the save, and the page's typed hook.

```rust
const PARAMS: &[ParamDecl] = &[
    ParamDecl::new(GAIN, "Gain", Kind::continuous(-60.0, 12.0, 0.1, None), 0.0).with_unit("dB"),
    ParamDecl::new(MODE, "Mode", Kind::Choice(&["Clean", "Warm", "Hot"]), 0.0),
    ParamDecl::new(BYPASS, "Bypass", Kind::Bool, 0.0).as_bypass(),
];
```

- The store keeps real units (dB, not 0 to 1). A host speaks in 0 to 1
  positions, and Seam converts both ways at its boundary.
- A choice's names are its list, and the host sees them as the parameter's text.
- `as_bypass` tells the exporter this parameter is the host's bypass. The
  processor still implements it: the host only learns which parameter it is.

## 2. Write the processor

[`Processor`](../crates/host/src/plugin.rs) is the audio thread's half, your
`processBlock`. The host calls `process` with a
[`Block`](../crates/host/src/audio.rs): the channels, edited in place, and the
block's MIDI events.

`GainProcessor` keeps the parameters' values in plain fields and reads them
again only when one moved. Each parameter has a change counter, so `retune`
sums the three counters and compares the sum with the last one it saw:

```rust
let seen = set
    .changes(self.gain_id)
    .wrapping_add(set.changes(self.mode_id))
    .wrapping_add(set.changes(self.bypass_id));
if self.seen == Some(seen) {
    return;
}
```

The processor holds an [`AudioParams`](../crates/params/src/set.rs), the handle
a block reads parameters through. Reading takes atomics only, and the audio
thread never allocates, locks or waits
([the invariants](../ARCHITECTURE.md#invariants)). The loop itself is plain DSP.

## 3. The plugin and its saved state

[`Plugin`](../crates/host/src/plugin.rs) is the main thread's half. Its
constants name the plugin (`INFO`) and list its `PARAMS`. `new` builds
everything the plugin owns; `activate` builds a fresh processor each time the
host activates it, so nothing the audio thread touches is allocated in a block.
`new` starts with the parameters, the session and the saved state over both:

```rust
let set = ParamSet::from_decls(PARAMS.iter().copied()).expect("valid declarations");
let session = Session::new(&set);
let saved = Arc::new(SavedState::new(&set, &session));
```

`new` returns `Self`, not a `Result`, so a failure here is a bug in the
plugin, and `expect` panics with its message. `saved` hands the exporter the
[`SavedState`](../crates/host/src/state.rs), and the exporter does every host
save and load through it without touching the plugin. The saved block is JSON:
the parameters' values, and one keyed section per session participant.

## 4. Undo

A `History` records changes to what you enroll in it:

```rust
let history = History::new(&ui, Bound::default()).expect("the first history");
history.enroll_params();
```

`ui` is the [`UiParams`](../crates/params/src/ui.rs) handle, the one an editor
writes parameters through. Only its writes record, as a user's: a host's
automation, a load and a learned controller never do, so undo never fights
the host. A drag is one [gesture](glossary.md#gesture) and so one undo step.

## 5. A value that is not a parameter

The label is state the host knows nothing about. Two things give it a life:

- A [`Value<String>`](../crates/params/src/value.rs) holds it. Enrolling it
  with `history.enroll(&label, "Rename")` makes its changes undo steps: the
  sets between two ticks are one step, the latest winning. A
  [`List`](../crates/params/src/list.rs)'s verbs are one step each instead.
- A [`Participant`](../crates/session/src/lib.rs) saves it. `capture` returns
  the value as a section; `apply` takes a restored section back, or `None`
  when the saved project predates the label, which must give the default.

The plugin registers the participant under a stable key, `"label"`, and the
session saves it there. Three details are easy to miss:

- The session holds a participant weakly, so the plugin keeps the `Rc` alive.
- A save writes the last capture, so after a change the plugin calls
  `Registration::changed`; `label_moved` does it after each set, undo and redo.
- A restore applies at the next tick, not when the host loads.

Undo replays a label step through the same `Value`, so `apply` needs no special
case for it.

## 6. MIDI learn

Learn adds three lines to `new`:

```rust
let (learn, _) = Learn::new(&set, Config::default());
let learn = Rc::new(RefCell::new(learn));
Learn::attach(&learn, &session_ui).expect("a free key");
```

and one to `activate`:

```rust
let learn = self.learn.borrow_mut().replace_audio();
```

`Learn::new` also returns an audio side, which `new` drops: `activate` takes a
fresh one each time with `replace_audio`, and hands the processor that
[`LearnAudio`](../crates/learn/src/audio.rs). `attach` registers learn's own
participant, so its mappings save under their own key. The processor calls
`learn.process(block.events)` at the top of `process`. Learned writes then
land before the parameters are read, in the block the controller arrived in.

To learn, the plugin calls `Learn::arm` on the gain's id with
`Takes::CONTROLLER`. The next controller the processor sees is captured on the
audio thread, and the next tick's poll commits it.

The plugin declares `midi_input: true` in its `INFO`. Open: no exporter
delivers MIDI events yet, so learn is exercised by driving the processor
directly, as the tests below do.

## 7. The idle tick, and its order

The exporter calls `Plugin::idle` about 60 times a second on the main thread.
Nothing of the plugin's runs there unless `idle` calls it:

```rust
fn idle(&mut self) {
    // Learn first: a mapping its poll commits reaches the session at the
    // session's next tick, which is this one.
    let _learned = self.learn.borrow_mut().poll();
    self.session.tick();
    self.history.tick();
    self.label_moved();
}
```

[`Plugin::idle`](../crates/host/src/plugin.rs) owns the order and its
reasons; the history's place in it does not matter.

## 8. Test it through a host

`seam_host::vst3::TestHost` drives the plugin through the VST3 exporter's own
interfaces, as a host would.
[`examples/gain/tests/gain.rs`](../examples/gain/tests/gain.rs) uses it for
what a host can reach: it sets up, activates and starts the host, then calls
`process_in_place` with changes given by host id, in the host's 0 to 1
(`normalize` converts dB first), reads the modes' names through `parameter`
and `text`, and carries the saved block through `save` and `load`.

Learn and undo are driven on the plugin itself, since a host cannot reach them:
the test arms the gain, feeds a controller through the processor, calls `idle`,
and saves and reloads to check the mapping survives.
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
Open: CI lays out and validates the Windows bundle only. The macOS bundle is
not built yet.

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
