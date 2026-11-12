# The decision record

Every decision Seam has taken and every rule its code keeps, each with its
reason. It began as the transfer from the C++/JUCE Seam (`nicvlav/seam-cpp`,
v0.7.0, commit `149005a`): what that library guaranteed, stated as invariants
with the reason each exists, in the order the Rust crates need them. No code
carries over; these rules do. They were read from the code and its tests, not
its docs, which had drifted in at least 25 places.

## Overview

**Verdict.** The C++ Seam's value comes down to about seventy rules. All of them
hold in Rust. Roughly a third of the C++ machinery existed only to survive
synchronous listeners, JUCE's threading model and C++ lifetimes, and the
poll-based Rust design drops it (listed in section 8). Every write carrying its
origin is the one structural change. In C++ the origin was implied by which
entry point a write used and observed through listeners. In Rust nothing
listens, so each write states its origin and the side effects follow from it.

**How to read it.** The decisions are numbered D1 onward. The table below gives
each one in a line: the question, what the Rust Seam does, and a status. Under
the table, one section per decision gives its full text: the question, what the
C++ Seam did, what Rust does, and what else was considered. How a decision was
reached and changed, up to the first commit on `main`, is in
[decision-history.md](decision-history.md), which its section links to; a later
change is in the commits that touched the section. A status of "decided" means
the owner decided or confirmed the choice. "To confirm" means the owner has not
confirmed it yet, and its history says when a later change moved its behaviour.
"Held" means the work does not need the decision yet, and its section keeps the
recommendation. A decision that names CLAP governs an exporter not built yet
(D68). The rules (P, U, S, L, H, B) state each invariant the code keeps and the
reason for it. To find the code behind a decision or a rule, read
`docs/decision-index.md`, which maps each id to the files, items and tests that
implement it, and which `scripts/check-decision-index.py` keeps true. Code and
comments never cite an id: they state the reason in words, and
`scripts/check-comment-tags.py` enforces that.

| Id | Question | Decision | Status |
| --- | --- | --- | --- |
| D1 | [Does a write by the plugin's own code tell the host? (P6)](#d1-a-plugins-own-write-and-the-host) | The write states one of two intents, and neither records undo. An edit reaches the host as one gesture; a sync changes values with no gesture and asks the host to re-read them. | decided |
| D2 | [Where does undo's before-state come from when enrollment happens mid-gesture? (U3)](#d2-undos-before-state-on-mid-gesture-enrollment) | The value when the gesture opened, which the Rust gesture record carries, so undo returns to the pre-drag value. A change from C++. | decided |
| D3 | [Undo or redo requested while a replay is applying? (U12)](#d3-undo-during-a-replay) | Refuse with `HistoryError::Replaying`. | decided |
| D4 | [Unknown parameter ids in a loaded block? (H2)](#d4-unknown-parameter-ids-in-a-loaded-block) | Keep them and save them back after the declared ones, like session sections. A load replaces the kept set, so the same block always gives the same state. | decided |
| D5 | [Must Rust read blocks saved by the C++ Seam? (H1)](#d5-reading-blocks-saved-by-the-c-seam) | No. The C++ Seam shipped in no product, and Rust reads only its own blocks. | decided |
| D6 | [A participant that registers after a restore that had no section for it. (S10)](#d6-a-participant-that-registers-after-a-restore) | Reset it to its defaults, once per key per restore, so a restore is a complete assignment for every key it lacked. A second participant that registers the same key keeps its state. A change from C++. | decided |
| D7 | [Restore neutrality for undo: a time window, or an origin on each write? (S12, U16)](#d7-restore-neutrality-for-undo) | Parameters restore with `Origin::Restore`, and undo reads the `RestoreClock`'s counts on every operation and clears when either has moved. Every history is neutral to restores; there is no opt-in. | decided |
| D8 | [History bound default. (U19)](#d8-the-history-bound) | Same unit, defined: the bytes a step's stored values keep alive plus a fixed overhead of 32 units, booked once, when it is recorded. 30,000 units and a floor of 30 entries. | decided |
| D9 | [How the learn dispatch table gets sequential consistency. (L4)](#d9-sequential-consistency-of-the-learn-dispatch-table) | A `triple_buffer` holds the published table: the audio thread takes the newest without waiting, locking or freeing, so Seam writes no `unsafe` there. | decided |
| D10 | [Learn over MIDI 2.0. (L2, L6)](#d10-learn-over-midi-20) | The UMP group and channel join the source. MIDI 1.0 values normalise centre-anchored and MIDI 2.0 values linear; native MIDI 2.0 assignable controllers are learnable, and the four-controller parser stays for MIDI 1.0 input. | decided |
| D11 | [At a gesture's close, does the drag's value overwrite a restore or host write that landed during the drag? (U1, U11, U15, U16, H3)](#d11-a-drags-close-and-a-restore-that-landed-during-it) | The restored or host value stands. The close records its step and writes nothing, so enrollment never changes how a value behaves. | decided |
| D12 | [Host step metadata for a stepped continuous range. (P17)](#d12-host-step-metadata-for-a-stepped-range) | Continuous to every host: VST3's step count and CLAP's stepped flag are set for bool and choice only. The JUCE adapter reports the range as not discrete with its real count. Not built yet: the C interface passes no step count for a continuous range, so through the adapter a stepped range has JUCE's default count. | decided |
| D13 | [When does a session restore apply to participants? (S5)](#d13-when-a-session-restore-applies) | Always on a later UI tick, whatever thread the restore came from. A change from C++. | decided |
| D14 | [What does a participant whose apply fails keep? (S7)](#d14-a-participant-whose-apply-fails) | Any failed apply keeps the participant's state and saves its raw section back unchanged. A change from C++. | decided |
| D15 | [Does the page get a parameter's state back after its own write? (B2)](#d15-the-pages-state-after-its-own-write) | Yes. The native side answers every `set` and `gestureEnd` with a `Param`, changed or not, so the page always ends on the store. | decided |
| D16 | [Where do wry's IPC handler, the custom-scheme asset server and "open the page in this window" live?](#d16-where-the-webview-pieces-live) | In `seam-webview`, a shell-tier crate not built yet. `seam-bridge` has no window or WebView dependency under any features. | decided |
| D17 | [Does learn take notes? (L2)](#d17-learning-notes) | Keep `Source::Note` for instruments that learn keys to parameters. A note-on writes its velocity, scaled as D10 scales a controller; a note-off writes nothing. | decided |
| D18 | [May earlier in-house code come into this repository, and in what form?](#d18-earlier-in-house-code) | An EQ and a VST3 shell from earlier in-house Rust work may seed the example plugin and the VST3 exporter, as generic Seam code under Seam's own names. Nothing else comes over. | decided |
| D19 | [Where do the restore counter and the applying-a-restore mark live? (U16, S6, S12)](#d19-the-restore-counter-and-the-applying-mark) | In a `RestoreClock` in `seam-params`: a count of restores stored and a count completed. The applying mark is the applying thread's own, so only that thread's writes record nothing. | decided |
| D20 | [What closes the window D13 opens between a restore's store and its apply? (U16, S5)](#d20-the-window-between-a-restores-store-and-its-apply) | Undo clears its history when either count has moved: at its first operation after a restore is stored, and again when that restore's apply completes. | decided |
| D21 | [What does a learn binding record of where its MIDI came from? (L2, L9)](#d21-what-a-learn-binding-records-of-its-midi-source) | The UMP group and the channel, each with −1 meaning any, in the source and in every saved `midiLearn` entry from schema 1. | decided |
| D22 | [How do controllers reach learn in VST3, which delivers them only as parameter changes? (L2, L3)](#d22-controllers-reaching-learn-in-vst3) | The VST3 exporter owns hidden proxy parameters in a reserved id range: a 16 × 130 grid through `IMidiMapping`, and a fixed pool for MIDI 2.0 assignable controllers. Still to build. | decided |
| D23 | [How does the processor send MIDI?](#d23-how-the-processor-sends-midi) | `Processor::process` takes one `Block`: the channels in place, the frame count, the events in, and an events-out list whose capacity `ProcessSetup::max_out_events` fixes at activation. | decided |
| D24 | [Who answers MIDI-CI when a plugin must answer for itself?](#d24-answering-midi-ci) | A VST3 host speaks MIDI-CI for the plugin, so the VST3 exporter projects declarations onto VST3's interfaces. Everywhere else a core-tier crate, `seam-ci`, answers, later. | decided |
| D25 | [Who owns the SysEx manufacturer ID that a MIDI-CI identity needs? (D24)](#d25-the-sysex-manufacturer-id) | None: a business answer. The working group's helper uses `0x7D`, which is reserved for non-commercial use. | held |
| D26 | [How does a debug build load the page from a dev server? (B5)](#d26-loading-the-page-from-a-dev-server) | `SEAM_PAGE_DEV_URL`, read in debug builds only: set, `seam-webview` loads that URL, so Vite's hot reload works; unset, it serves the bundled page. | decided |
| D27 | [What runs the generator? (B3)](#d27-what-runs-the-generator) | An xtask: `cargo xtask gen` writes every generated file, and `cargo xtask bundle` is to run the generator, the page build and the plugin build in that order. Not built yet: `bundle`. | decided |
| D28 | [Which threads may make a program write? (P6, P11)](#d28-threads-that-may-make-a-program-write) | An edit runs on the UI thread only and joins the parameter's gesture count. A sync runs on any thread but audio. | decided |
| D29 | [Do plain values have gestures? (U3, U4, U5, U16, L7)](#d29-gestures-on-plain-values) | Yes. A plain value has its own gesture count and token, so one drag on any control is one undo step. | decided |
| D30 | [An NRPN data entry sent with only its high byte. (L2)](#d30-an-nrpn-data-entry-with-only-its-high-byte) | A new high byte dispatches the 14-bit value with the low byte at 0, and a low byte that follows refines it. A binding whose capture saw no low byte scales the high byte as 7-bit instead. | decided |
| D31 | [What identifies a parameter to VST3, CLAP and AU hosts: its declaration index or a hash of its key? (P10, H5)](#d31-what-identifies-a-parameter-to-hosts) | A hash of the key, so a later version of a plugin can insert, reorder, regroup or remove parameters without orphaning automation. The hash function is fixed forever. | decided |
| D32 | [When does the host hear that a user's gesture began? (P6, P11)](#d32-when-the-host-hears-a-gesture-begin) | When it opens, so a touch held still reaches the host and Touch and Latch modes hold the value. A program edit and a replay open only on a changing write. | decided |
| D33 | [Undo or redo requested while a user gesture is open? (U8, U11, U12)](#d33-undo-during-a-user-gesture) | Refused with `HistoryError::GestureOpen` until the gesture closes. | decided |
| D34 | [Does a sync reach surfaces while a gesture is open? (P6, P12)](#d34-a-sync-while-a-gesture-is-open) | No: it waits with the host-shaped writes and shows at the first poll after the close. | decided |
| D35 | [How does a reserved NRPN block dispatch? (L2, L9)](#d35-a-reserved-nrpn-block) | A mapping whose group and channel could hear a reserved number is refused. A saved entry that a later reservation blocks drops at restore like any bad entry. | decided |
| D36 | [In what order does one undo tick record the changes it sees? (U5, U14)](#d36-the-order-one-undo-tick-records) | In the order they happened: every gesture record, plain value commit and list verb takes a number from one sequence on the `RestoreClock`, and the tick records by it. | decided |
| D37 | [Does the history keep enrolled values and lists alive? (U13)](#d37-whether-the-history-keeps-enrolled-values-alive) | No. A tracker holds its value or list by a `Weak` handle, so dropping the last handle frees it, and the history forgets the enrollment at its next tick. | decided |
| D38 | [Do parameters and lists carry the origin of their newest write? (U10)](#d38-the-origin-of-a-newest-write) | Yes, beside the change counter, as a plain value does. A value derived from a parameter or list passes the origin on, and a derived change records nothing. | decided |
| D39 | [What stops a load on another thread landing between a replay's check of the restore clock and its write? (U16, H3)](#d39-a-load-between-a-replays-check-and-its-write) | A lock in the `RestoreClock`. A `Load` takes it for each parameter write and moves the stored count under it, and each replay step that writes parameters holds it across its check of the stored count and its write, so no undo step for a change finished before a load replays over it. A drag open across the load, a lone write included, keeps its pre-drag before, and undoing it writes that value (D11). | decided |
| D40 | [How many undo histories may one `UiParams` have? (U1)](#d40-one-undo-history-per-uiparams) | One. A second history over the same handle is refused with `HistoryError::HistoryExists`. | decided |
| D41 | [A list step replays while a verb from another thread waits for the tick: what then? (U17)](#d41-a-list-step-replaying-against-a-waiting-verb) | The step drops the waiting verb, which then records nothing, and moves the list's generation, so every step recorded before replays as nothing. | decided |
| D42 | [Does a change another thread makes while a tick runs keep its order? (D36, U14)](#d42-order-across-ticks) | Yes. The tick takes a number from the sequence as it starts and records only changes numbered before it; later ones wait for the next tick, in order. | decided |
| D43 | [Does a change derived from a recorded change record a step of its own? (U10, D38)](#d43-a-derived-change-records-nothing) | No. `Value::set_from` and `List::derived` never record, whatever origin they pass on, so one user action stays one step. | decided |
| D44 | [A participant tells the session it changed from its own method, while its owner holds it mutably: what then? (S4)](#d44-a-participant-that-changes-while-its-owner-holds-it) | `Registration::changed` marks the registration held up, and the next `SessionUi::tick` captures it before its apply pass, a tick made inside a held-up capture included; a save shows the change one tick later. | decided |
| D45 | [What happens to registrations when the `SessionUi` they were made through drops? (S2)](#d45-registrations-when-a-sessionui-drops) | They end, as if each `Registration` had dropped: their sections stay in every save, their keys are free for a later handle, and `connected` turns false. | decided |
| D46 | [What does a derived write do to a plain value's open gesture? (D43, U15, U16)](#d46-a-derived-write-and-a-plain-values-open-gesture) | By its origin. A restore or a replay rebases the gesture; a write derived from a user's or the plugin's own change joins it; one derived from a host or learned change leaves it alone. | decided |
| D47 | [When does a lone NRPN high byte settle as coarse? (L3, D30)](#d47-when-a-lone-nrpn-high-byte-settles-as-coarse) | When a learn poll finds the same unsettled capture an earlier poll found, and a whole audio block after the one that staged it has ended. A second high byte with no low byte between still settles it at once on the audio thread. | decided |
| D48 | [How does a plain `Value` serve as a learn `Target`? (L7, D29, D33)](#d48-a-plain-value-as-a-learn-target) | Open. `Value` reports only whether some gesture is open, so `Target::held` cannot leave out learn's own burst. Needed before a plugin learns into a plain value. | held |
| D49 | [May a session participant write a parameter through the UI handle while it applies? (U16, P6)](#d49-a-participant-writing-a-parameter-during-apply) | No: it writes through `ProgramParams::sync`, which records nothing and tells the host no gesture. A user or edit write through `UiParams` while the applying mark is up fails a debug assertion. | decided |
| D50 | [How does each activation's processor get learn's audio side? (L5)](#d50-learns-audio-side-per-activation) | `Learn::replace_audio()` makes a fresh one that reads the current mappings; the one handed out before keeps its last table and sees no later edit. | decided |
| D51 | [Must a plugin be `Send`? (U5, S6, L7, H1)](#d51-whether-a-plugin-must-be-send) | No. The exporter calls the plugin and drops it on its main thread only. Only a plugin that is `Send` may leave that thread: the C interface binds it to no thread. | decided |
| D52 | [Who makes the `RestoreClock`? (D19, D20, D39)](#d52-who-makes-the-restoreclock) | The `ParamSet`: `ParamSetBuilder::build` makes one, and nothing else can, so every history, value, list, session and load of one plugin shares one clock by construction. | decided |
| D53 | [What ticks a plugin with no editor open? (U5, S6, L7, D47)](#d53-what-ticks-a-plugin-with-no-editor-open) | One main-thread timer per instance, at 60 Hz for the instance's life, editor open or not, ticks the plugin's parts and then calls `Plugin::idle`; the editor never ticks. The VST3 exporter and the JUCE adapter supply the timer; CLAP's exporter is to use the host's timer-support extension, and a CLAP host may lack timer support (still to settle). | decided |
| D54 | [What does detaching a learn target do to its mapping? (L8, L9)](#d54-detaching-a-learn-target) | Holds it. `Learn::detach` ends an arm on the target, closes its open burst once and drops the target, and its mapping becomes a held entry under its key. `unmap` is what removes a mapping. | decided |
| D55 | [What happens to a participant whose apply panics? (S7, D14)](#d55-a-participant-whose-apply-panics) | Catch and hold: the session contains the panic as it does a failed apply, and ignores the participant's captures until a later apply succeeds, so a half-built state is never saved. | decided |
| D56 | [Which hash names a parameter to hosts, and which ids does D22 reserve? (D31, P10)](#d56-the-hash-that-names-a-parameter) | 32-bit FNV-1a over the key's UTF-8 bytes with the top bit cleared. The builder refuses a key whose id another key already has, or that falls in `0x7FFF_0000` to `0x7FFF_FFFF`, the ids D22's proxies take. | decided |
| D57 | [From what does a stepped range lay out its lattice? (P2, P4)](#d57-the-lattice-of-a-stepped-range) | From the decimals the range was written with, in whole numbers of its finest place, so every point of a decimal range, 0 dB included, lands on the nearest `f32` to its decimal. | decided |
| D58 | [What does the plugin trait tell every host, beyond its parameters and state? (H5)](#d58-what-the-plugin-trait-tells-every-host) | `PluginInfo` gains the vendor's addresses, a description, `Feature`s and the main bus `Layout`s; `Plugin` gains `latency`, `tail` and `deactivate`; `Processor::process` returns a `Status`. Deferred: whether the host is rendering offline or in real time, which `ProcessSetup` has no field for. | decided |
| D59 | [What shape does the VST3 exporter give a plugin? (D51, D53, P7, C2)](#d59-the-shape-of-the-vst3-exporter) | One object that is both the audio processor and its own edit controller, a thin shell over an exporter core it shares with the C interface. The plugin is built at `initialize`, on the thread that calls it. No MIDI events are delivered yet. | decided |
| D60 | [What happens when plugin code panics inside a host's call? (P9)](#d60-a-panic-inside-a-hosts-call) | It is caught at every entry point that can panic and never unwinds into the host. A panic in the processor silences the instance's output until the next activation. | decided |
| D61 | [Who flushes denormals to zero? (the comparison's fairness row)](#d61-who-flushes-denormals) | Every exporter, around every `process` call: FTZ and DAZ on x86, FZ on AArch64, restoring the host's setting after, so no plugin has to. | decided |
| D62 | [What shape is the C interface the JUCE adapter is driven by? (H5, D51, D60)](#d62-the-c-interface-for-the-juce-adapter) | One exported symbol, `seam_plugin`, answering a table of function pointers, as CLAP's entry point does: the plugin's layouts and every call into an instance, each with the thread it may run on. No MIDI passes yet; it arrives as UMP both ways with the exporters' MIDI input. | decided |
| D63 | [How is the C header kept true to the Rust side? (D27)](#d63-keeping-the-c-header-true) | Seam's own generator writes `crates/host/include/seam.h` from the Rust declarations, and the header asserts every struct's size and every field's offset as Rust laid them out. | decided |
| D64 | [How does the JUCE adapter build a plugin? (H5, D18)](#d64-how-the-juce-adapter-builds-a-plugin) | It links the plugin crate as a static library built with only its `cabi` feature. `cargo xtask gen` writes a `seam-juce.cmake` per plugin, and `seam_add_juce_plugin` runs cargo and adds the adapter's one processor class. | decided |
| D65 | [What does a host see of a plugin through JUCE? (D31, D53, D58, C4)](#d65-what-a-host-sees-through-juce) | What JUCE 9 makes of it, as of the JUCE EQ; checked with VST3 on Windows. Parameter ids are JUCE's hash of the key, not D31's, and the one program has the plugin's name. | decided |
| D66 | [What does a load do with bytes after the block, and is the host told that a load failed? (H2)](#d66-bytes-after-the-block-and-a-failed-load) | Trailing NUL bytes are trimmed and the block loads, and JSON whitespace around the object loads too; any other byte after the object makes the block unrecognised. A load that fails is answered as failed, except through JUCE, whose `setStateInformation` answers nothing, so the adapter cannot tell its host. | decided |
| D67 | [How are the rules checked that keep locking types off the audio thread? (P6, D39)](#d67-checking-the-audio-thread-locking-rule) | By a debug-build mark, `seam_params::audio_thread`: a per-thread depth an exporter raises around everything its `process` call runs, and every call that locks asserts the mark is absent. | decided |
| D68 | [Which of the 2026-11-05 audits' items wait until their code exists?](#d68-audit-items-that-wait-for-their-code) | Those whose code has not started: the CLAP exporter, `seam-webview`, the native side of the page protocol, automation smoothing and splitting, and semver checks. | decided |
| D69 | [Where is a continuous range checked, and how does a declaration that is a constant build one? (P2, P10, D57)](#d69-where-a-continuous-range-is-checked) | When it is made. `Range::new`, a `const fn`, returns a `RangeError` for an invalid range, and `Kind::continuous`, a public `const fn`, panics on what it refuses. A typed declaration's range is checked by `Range::new` too: by `FloatDecl::checked` while compiling, and when it is added. | decided |
| D70 | [How does learn read MIDI 1.0's paired controllers and parameter numbers where MIDI 1.0 leaves room? (L2, L3, D30)](#d70-learns-reading-of-midi-10-pairs) | Per group and channel, the two NRPN address bytes pair in either order, 127 with 127 selects nothing, and data increment and decrement are never learned. A controller 32 to 63 right after the controller 32 below it learns that one as a 7-bit controller. A learned pair plays at 7 bits: 14-bit controller playback is open, to be built when an exporter delivers MIDI. | decided |
| D71 | [What does learn route after a restore whose `midiLearn` section it cannot read? (L9, D14, H2)](#d71-learn-after-an-unreadable-midilearn-section) | Nothing from before it. Learn ends the arm, clears every mapping and held entry and publishes the empty table, then closes open bursts (D82) and fails the apply, so the session keeps the section. | decided |
| D72 | [How much may a host ask of an instance? (H2, D59, D62)](#d72-how-much-a-host-may-ask) | Fixed limits: a processor is built for at most 65,536 frames a block, a setup that promises more being built for that many, a saved block is at most 64 MiB, and a block's automation has room for 8 points per parameter on average, shared by its queues. | decided |
| D73 | [What does an activation make of what the host's setup leaves open? (D58, D59, D62)](#d73-what-an-activation-makes-of-an-open-setup) | An activation while active builds a fresh processor and hands the old one back to the plugin, in both exporters. A setup whose block size is 0, or negative in VST3, takes 1,024 frames. | decided |
| D74 | [Which plugins and platforms does `export_vst3!` refuse to build? (D53, D58)](#d74-what-export_vst3-refuses-to-build) | A main bus wider than stereo, any platform but Windows and macOS, a plugin that declares no layout, and a crate built with `panic = "abort"`. | decided |
| D75 | [How does a fault the exporters catch become visible outside a test? (D60)](#d75-making-a-caught-fault-visible) | It is counted, and its kind reported once. Each instance counts its faults from every thread, wait-free, and the first fault of each kind in an instance is written as one line. | decided |
| D76 | [What does a panic leave queued while the exporter tells the host of edits? (P9, P11)](#d76-a-panic-while-edits-are-told-to-the-host) | Everything not yet told: the parameter keeps the flags whose events it had not handed out and the queue is marked to be scanned again, so the host still hears a balanced bracket. | decided |
| D77 | [How deep may a session section nest? (S1, S4, H2)](#d77-how-deep-a-session-section-may-nest) | 125 levels (`seam_session::MAX_SECTION_DEPTH`). A deeper capture is refused as it is taken, so a save never writes a block the same build cannot load. A capture that would grow the block past `MAX_STATE_BYTES` is refused too, against a budget the saved state sets whenever it is made, and a load whose block would save back past it is refused whole. | decided |
| D78 | [How do a page and a native side of different protocol versions find out? (B1, B4)](#d78-protocol-versions) | The native side answers every `ready`: with a snapshot when it speaks the page's version, and otherwise with a `reject` carrying its own version and a reason, which fails the page. No native side exists yet; the decoding and the page's half are built. | decided |
| D79 | [What does the page client do while the native side has not answered, and when a hook's command changes? (B1, B5)](#d79-the-page-client-before-the-native-side-answers) | The client sends `ready` again every second until a snapshot or a reject arrives, it fails, or it is disposed. `useCommand` keys its pending count and its last result and error by the command's name. The page holds its writes until it is answered, so each `ready` may start a new page session. | decided |
| D80 | [How does a plugin upgrade a block an older version of it saved? (H1, H2)](#d80-upgrading-an-older-saved-block) | It declares `Plugin::SCHEMA_VERSION` and `Plugin::migrate`, which Seam attaches to the saved state it builds: a block of an older plugin version (D84) is handed to `migrate` whole, as a JSON object, to rewrite in place before it loads, or to refuse with its reason (D85). A save keeps the unknown parameter and top-level keys the older build does not read. | decided |
| D81 | [Does a late apply made while a panic unwinds leave the session owing a pass? (S6, S10, D55)](#d81-a-registration-made-while-a-panic-unwinds) | No. Only a panic that began inside a pass or a late apply leaves the pass owed, for the next tick to run again; a registration made from a destructor during another panic owes none, so it clears no undo history. | decided |
| D82 | [When do a restore and an unmap run a learn target's own code? (L8, L9, D54, D71)](#d82-when-learn-runs-a-targets-own-code) | Last: after learn has settled its table, published it and told the session, so a target whose `end` panics leaves no earlier mapping routing, in learn or on the audio side, and learn usable. | decided |
| D83 | [Do a page control's `onDrag` and `setValue` differ? (B2)](#d83-a-controls-drag-and-one-shot-set) | Only in the gesture `onDragStart` and `onDragEnd` put around a drag: they are one function, so a `setValue` during a drag joins it. | decided |
| D84 | [Whose version does the saved block carry? (H1, H2, D80)](#d84-the-saved-blocks-two-versions) | Two, each its own: `seamStateVersion` is Seam's frame, at 1, and `pluginStateVersion` the plugin's schema, 0 with no migration and when absent. A save writes both, each the higher of the block's and the build's. | decided |
| D85 | [How does a migration say it cannot upgrade a block? (D80)](#d85-a-migration-that-cannot-upgrade-a-block) | It returns a `Result` whose error is a boxed `Error`, so it can use `?`, and `LoadError::MigrationFailed` carries the reason. | decided |
| D86 | [How does `PluginInfo` hold the AU codes? (D64)](#d86-the-au-codes) | As one `AuCodes` with named `subtype` and `manufacturer` fields, so the two cannot be swapped unseen. | decided |
| D87 | [How does `ProcessSetup` give the main bus? (D58)](#d87-the-main-bus-in-processsetup) | As one `layout: Layout`, the one of the plugin's layouts the host chose. | decided |
| D88 | [What does a test host answer for a call the plugin refused? (D60)](#d88-a-call-a-test-host-was-refused) | `Err(Refused)`: the VST3 host's carries the `tresult` it was answered, the C interface's stands for its `false`. | decided |
| D89 | [How does plugin code name a parameter and read its value? (P1, P10)](#d89-how-plugin-code-names-a-parameter) | As a typed parameter, `Float`, `Bool` or `Choice<E>`, that carries its store and is read with `get`, as an `f32`, a `bool` or the plugin's enum, on any thread. `ParamSet::bind` makes one by key, once, and checks the declaration. `Changes` holds a stamp and asks once per block whether any of the parameters it is given moved. | decided |
| D90 | [How does a plugin declare its parameters? (P1, P10, D89)](#d90-how-a-plugin-declares-its-parameters) | In one function that returns them: `Params::declare` adds each through a `Declarer`, and a typed declaration, `FloatDecl`, `BoolDecl` or `ChoiceDecl<E>`, hands back its kind of parameter. A mistake is a compile error where it can be, through `checked`, and otherwise one `DeclError` when `ParamSet::declare` builds the set. | decided |
| D91 | [Who builds a plugin's parameter set, session and saved state? (D51, D52, D53, D90)](#d91-who-builds-a-plugins-parts) | Seam, once per instance: `Parts::for_plugin` declares the plugin's `Params` and builds the set, the session and the saved state over it, and `Plugin::new` takes the parameters and the `Parts`. The plugin owns its parts, and Seam ticks them before each `idle` through a `Driver`, a link that does not keep them alive; a plugin never ticks them, and a test with no host ticks through a `Driver`. | to confirm |
| D92 | [When does a plain value saved through `Parts::saved_value` reach the session? (D44, D91)](#d92-when-a-saved-plain-value-is-captured) | At the tick: `Driver::tick` ends with `Parts::sync`, which captures each saved value whose change count moved, and skips one whose newest write was its own apply's, which the session's apply pass captured already. A save between a change and the next tick holds the value from before the change. | decided |
| D93 | [How does a plugin declare a struct of parameters without writing `declare`? (D90)](#d93-a-derive-that-writes-declare) | `#[derive(Params)]`, under `seam-params`'s `derive` feature: a `#[param(...)]` mark on each field holds the declaration's arguments, then `name = value` options and lone-word flags, and the field's type names the kind through `Declared`. Each declaration is a named constant ending in `checked`, so `cargo check` refuses a mistake on the mark, two fields with one key included. | decided |
| D94 | [How does a plugin name a choice's values without writing `Choices`? (D89, D93)](#d94-a-derive-that-writes-choices) | `#[derive(Choices)]`, under the same `derive` feature: the labels and both mappings in the order of the variants, a label being the variant's name unless a `#[choice("...")]` mark gives one. A written discriminant must be the variant's position, and a bad label is refused on itself: `cargo check` evaluates both checks. | decided |
| D95 | [When does a call at the plugin boundary panic, and when does it return a `Result`? (D90, D91)](#d95-mistakes-panic-failures-return-a-result) | A mistake fixed in the plugin's source, such as a call made twice or a switch turned on without what it needs, panics with what to change; a failure that can depend on data at run time returns a `Result`. So `Parts::undo` and `Parts::midi_learn` return no `Result`, and `Parts::saved_value` keeps one. | decided |

**Open.** D25 and D48 are held until their stage needs them: D25 `seam-ci`, D48
the first plugin that learns into a plain value. Each keeps its recommendation
until decided, and no rule depends on it.

The rest default without asking: host and learn writes become visible to every
poller even with no editor open (P13); the non-finite table stays per origin
(P3), since hosts can observe it; a menu prefix collision is an error (M2); a
JSON `null` section means absent (S7); collections undo as owned lists only,
with no passive snapshot retrofit (U17); every learn defect still open in v0.7.0
is fixed rather than reproduced (L2, L3, L6, L7, L8, L9, L10, H4).

**What blocks what.** P1 (store) and P2 (snapping) come first; nearly every rule
is phrased in terms of them. P6 (origins) gates undo's recorder API, since undo
can only learn of writes from the write itself. P11 (gestures) gates P12, undo's
one-step rule and the learn relay. Session S5 to S10 gate undo's restore
neutrality (U16). The learn dispatch table (L4) depends only on P1 and P8 and
can be built in parallel with undo; everything else in learn sits on it.

**Bugs in the C++ Seam this design must not repeat** (section 9 has all of
them): a shared parameter loses host updates when one of its two views is
destroyed (no per-target registration count); when a range's end sits less than
half a step past the last lattice point, writing the end, a host 1.0 or a reload
lands on that point instead; views keep the announced value instead of the
stored one; the learn dispatch table's publish and lease can reorder and free a
snapshot the audio thread is reading; nested undo during a replay corrupts the
history; and the learn host relay can overwrite a newer value.

## The decisions in full

One section per decision, in order. Each gives the question, what the C++ Seam
did, what Rust does and what else was considered, and links its history when it
has one.

### D1. A plugin's own write and the host

**Question and C++ Seam.** Does a write by the plugin's own code tell the host?
(P6) Only when it happened to run on the UI thread, and then a value with no
begin or end.

**Rust.** The write states one of two intents, and neither records undo. An edit
reaches the host as one gesture; several values changed together share it. A
sync (preset recall, randomise) changes values with no gesture and asks the host
to re-read them (VST3's values-changed restart, CLAP's value rescan), so no
automation lane is touched.

### D2. Undo's before-state on mid-gesture enrollment

**Question and C++ Seam.** Where does undo's before-state come from when
enrollment happens mid-gesture? (U3) The value at enrollment; tests pin it.

**Rust.** The value when the gesture opened (U2 allows it). The Rust gesture
record carries it, and undo returns to the pre-drag value. A change from C++.

### D3. Undo during a replay

**Question and C++ Seam.** Undo or redo requested while a replay is applying?
(U12) Unguarded. A nested undo moves the cursor twice and marks a step undone
without replaying it; a nested redo moves it +1 then −1, so the undone entry
lists as applied and the redone one as undone (`juce_UndoManager.cpp:254-271`).

**Rust.** Refuse with `HistoryError::Replaying` (U12).

### D4. Unknown parameter ids in a loaded block

**Question and C++ Seam.** Unknown parameter ids in a loaded block? (H2)
Dropped, and gone at the next save, while unknown session sections are kept.

**Rust.** Keep them and save them back after the declared ones, like session
sections. A load replaces the kept set, so the same block always gives the same
state (H2).

### D5. Reading blocks saved by the C++ Seam

**Question and C++ Seam.** Must Rust read blocks saved by the C++ Seam? (H1) Not
applicable.

**Rust.** No. The C++ Seam shipped in no product: its only users were internal
validation plugins, which saved their own state format. Rust reads only its own
blocks, so JUCE's VST3 and AU parameter-id derivation and state containers need
no matching.

### D6. A participant that registers after a restore

**Question and C++ Seam.** A participant that registers after a restore that had
no section for it. (S10) Keeps whatever it holds.

**Rust.** Reset it to its defaults, once per key per restore (the restore
records which keys it lacked, and the first participant to register such a key
takes its reset), so a restore is a complete assignment for every key it lacked.
A second participant that registers the same key after that keeps its state. A
change from C++.

[History](decision-history.md#d6-a-participant-that-registers-after-a-restore)

### D7. Restore neutrality for undo

**Question and C++ Seam.** Restore neutrality for undo: a time window, or an
origin on each write? (S12, U16) An opt-in window that recorders watch, with
documented side effects (an unrelated open drag on a plain value is rebased; a
recorder connected, in signal order, after an open listener that registers a
participant records that participant's apply).

**Rust.** Parameters restore with `Origin::Restore`; the session sets the
`RestoreClock`'s applying mark around each participant's apply (D19); undo reads
the clock's counts on every operation and clears when either has moved (D20).
Every history is neutral to restores; there is no opt-in, and no option keeps
history across a restore (C++ had one, `keepHistory`,
`seam_UndoRecorder.h:1626-1636`).

### D8. The history bound

**Question and C++ Seam.** History bound default. (U19) A 30,000-unit cap with a
floor of 30 transactions; the comment claims unbounded and that 0 means no
limit, but JUCE raises 0 to 1. A step's units were its action's size plus its
two payloads, which the action's size already holds, so payloads counted twice;
heap was excluded.

**Rust.** Same unit, defined: the bytes a step's stored values keep alive plus a
fixed overhead of 32 units, booked once, when it is recorded. What a value keeps
alive is its `UndoSize`, a trait of `seam-undo` apart from `UndoEq` that every
enrolled value and list element implements: `size_of_val` by default, and for
the standard strings, vectors, boxes, shared pointers and collections their heap
too, a vector by its capacity, so a 64 KiB `Vec<u8>` books its 64 KiB.

A list's insert books only the element's handle, since the element stays in the
list until an undo moves it into the step; a remove or a clear books the
elements it holds. 30,000 units and a floor of 30 entries, documented as what
they are. A parameter step books 40 units, so about 750 fit, close to what C++
held for float steps; pruning drops whole entries, and the floor keeps 30
however large they are, so 30 steps of a 64 KiB value keep about 4 MiB.

**Alternatives.** The alternatives were a size function passed at enrollment,
with a byte cap that overrides the floor, or sizing folded into `UndoEq`.

[History](decision-history.md#d8-the-history-bound)

### D9. Sequential consistency of the learn dispatch table

**Question and C++ Seam.** How the learn dispatch table gets sequential
consistency. (L4) Acquire/release, which is not enough.

**Rust.** A `triple_buffer` holds the published table: the audio thread takes
the newest without waiting, locking or freeing, and a replaced table drops on
the UI thread at a later publish, so Seam writes no `unsafe` there.

**Alternatives.** Had that failed, the hand-written form: SeqCst fences on both
sides, the form `loom` can check (L4), with the loom models in the table's first
commit.

[History](decision-history.md#d9-sequential-consistency-of-the-learn-dispatch-table)

### D10. Learn over MIDI 2.0

**Question and C++ Seam.** Learn over MIDI 2.0. (L2, L6) MIDI 1.0 only: CC 64
maps to 0.50394; NRPN assembled from four controllers.

**Rust.** The UMP group and channel join the source (D21). MIDI 1.0 values
normalise centre-anchored, as the MIDI 2.0 specification translates them: `v ≤
c` gives `v / (2c)`, otherwise `0.5 + (v − c) / (2(max − c))` (7-bit: c = 64,
max = 127; 14-bit: c = 8192, max = 16383), so 0, the centre and the maximum land
exactly on 0, 0.5 and 1, and a controller reads the same, within 1.3e-5, whether
it arrives as MIDI 1.0 or in a host's MIDI 2.0 translation.

MIDI 2.0 values are linear, `v / max`, within 1e-5 of centre-anchoring at 16
bits and 1.2e-10 at 32. Native MIDI 2.0 assignable controllers are learnable,
and the four-controller parser stays for MIDI 1.0 input. Pitch bend is not
learnable; if it becomes learnable, it is centre-anchored at every width, since
its centre is its rest position.

[History](decision-history.md#d10-learn-over-midi-20)

### D11. A drag's close and a restore that landed during it

**Question and C++ Seam.** At a gesture's close, does the drag's value overwrite
a restore or host write that landed during the drag? (U1, U11, U15, U16, H3) Yes
on an undo-enrolled parameter: recording the step applied it, which wrote the
drag's last value over the store inside one extra host begin and end
(`seam_UndoRecorder.h:146-149`, `:172`, `:834`). Not enrolled, the restored or
host value stood.

**Rust.** The restored or host value stands. The close records its step and
writes nothing, so enrollment never changes how a value behaves.

### D12. Host step metadata for a stepped range

**Question and C++ Seam.** Host step metadata for a stepped continuous range.
(P17) Continuous to VST3 and AU: the range was not discrete, and JUCE sends
VST3's step count and AU's integer range only for a discrete parameter. AAX got
the count.

**Rust.** Continuous to every host. VST3's step count and CLAP's stepped flag
are set for bool and choice only. The JUCE adapter reports the range as not
discrete with its real count, so AAX gets what it got before. Not built yet: the
C interface passes no step count for a continuous range, so through the adapter
a stepped range has JUCE's default count.

[History](decision-history.md#d12-host-step-metadata-for-a-stepped-range)

### D13. When a session restore applies

**Question and C++ Seam.** When does a session restore apply to participants?
(S5) Inline, before `restore` returned, when called on the UI thread with no
pass running (`seam_SessionRegistry.h:755-756`); from a posted hop otherwise.

**Rust.** Always on a later UI tick, whatever thread the restore came from. A
change from C++.

### D14. A participant whose apply fails

**Question and C++ Seam.** What does a participant whose apply fails keep? (S7)
The raw section only when the apply, or the capture after it, threw. The
contract and the shipped participant reset to defaults on data that did not
decode, and the next save wrote the defaults over it.

**Rust.** Any failed apply keeps the participant's state and saves its raw
section back unchanged. A change from C++.

### D15. The page's state after its own write

**Question and C++ Seam.** Does the page get a parameter's state back after its
own write? (B2) No. A write that changed nothing returned early, and the echo of
one that did was suppressed, so the page never learned the snapped value of its
own write.

**Rust.** Yes. The native side answers every `set` and `gestureEnd` with a
`Param`, changed or not, so the page always ends on the store.

### D16. Where the WebView pieces live

**Question and C++ Seam.** Where do wry's IPC handler, the custom-scheme asset
server and "open the page in this window" live? Not applicable.

**Rust.** In `seam-webview`, a shell-tier crate not built yet. `seam-bridge` has
no window or WebView dependency under any features.

[History](decision-history.md#d16-where-the-webview-pieces-live)

### D17. Learning notes

**Question and C++ Seam.** Does learn take notes? (L2) No: control changes and
NRPNs only (`seam_MidiSource.h:13-14`).

**Rust.** Keep `Source::Note` for instruments that learn keys to parameters. A
note-on writes its velocity, scaled as D10 scales a controller: centre-anchored
for MIDI 1.0, linear for MIDI 2.0's 16-bit velocity (the attribute beside it is
not velocity). A note-off writes nothing, so the value holds as a controller's
does. A MIDI 1.0 note-on at velocity 0 is a note-off; a MIDI 2.0 one is a
note-on. An arm states whether it takes a note, so a key played while learning a
controller does not take the arm.

A glide to the new value is the plugin's, not Seam's.

### D18. Earlier in-house code

**Question and C++ Seam.** May earlier in-house code come into this repository,
and in what form? Not applicable.

**Rust.** An EQ and a VST3 shell from earlier in-house Rust work may seed the
example plugin and the VST3 exporter. What lands is generic Seam code under
Seam's own names: no product names, branding, presets or product-specific
controls, and no references back to the projects it came from. Nothing else
comes over. The example plugin is a generic EQ.

[History](decision-history.md#d18-earlier-in-house-code)

### D19. The restore counter and the applying mark

**Question and C++ Seam.** Where do the restore counter and the
applying-a-restore mark live? (U16, S6, S12) A window signal the session emitted
and recorders watched.

**Rust.** In a `RestoreClock` in `seam-params`: a count of restores stored and a
count of restores completed. The applying mark is the applying thread's own: a
thread-local list of the clocks that thread is applying a restore for, each with
its guards' depth. Only the thread that took the mark records nothing, so a
worker thread's write while the UI thread applies records as the worker's.

The session writes the counts and takes the mark (and the saved block's load
bumps the stored count, H3), undo reads both, and both take the clock from the
`ParamSet` that owns it (D52), so undo and session never depend on each other.

**Alternatives.** The alternative was the applying thread's id stored in the
clock.

[History](decision-history.md#d19-the-restore-counter-and-the-applying-mark)

### D20. The window between a restore's store and its apply

**Question and C++ Seam.** What closes the window D13 opens between a restore's
store and its apply? (U16, S5) A restore on the UI thread applied inline, so
only an off-thread restore had the window.

**Rust.** Undo clears its history when either count has moved: at its first
operation after a restore is stored, and again when that restore's apply
completes, which drops any step recorded in between; a pending restore outranks
live edits (S8).

### D21. What a learn binding records of its MIDI source

**Question and C++ Seam.** What does a learn binding record of where its MIDI
came from? (L2, L9) The channel alone, −1 meaning any; MIDI 1.0 has no group.

**Rust.** The UMP group and the channel, each with −1 meaning any, in the source
and in every saved `midiLearn` entry from schema 1, so two controllers on
different groups stay apart and no migration follows. In VST3 the group is the
event bus index, as the MIDI 2.0 working group's helper reads it.

### D22. Controllers reaching learn in VST3

**Question and C++ Seam.** How do controllers reach learn in VST3, which
delivers them only as parameter changes? (L2, L3) JUCE's wrapper added a hidden
parameter per channel and controller (16 × 130: the 128 control changes, channel
pressure, pitch bend) whenever the plugin took MIDI input
(`JUCE_VST3_EMULATE_MIDI_CC_WITH_PARAMETERS`,
`juce_audio_plugin_client_VST3.cpp:104-108`). No `IMidiMapping2`, so no MIDI 2.0
assignable controllers.

**Rust.** The VST3 exporter owns hidden proxy parameters in a reserved id range
that no key's hash may land in (D31): the same 16 × 130 grid, mapped through
`IMidiMapping`, and a fixed pool for MIDI 2.0 assignable controllers. While a
target is armed, `IMidiLearn2` binds the controller it reports to a free proxy,
published through `IMidiMapping2` and
`restartComponent(kMidiCCAssignmentChanged)`; capture still happens on the audio
thread, from that proxy's next change.

The exporter turns each proxy change into a UMP at its offset, so learn has one
input path, and a proxy is never a target, so a learned write never reaches the
host as an edit. A product's own controller block (a fixed NRPN range, say) is
pre-mapped by that plugin, not built into Seam. Still to build: it comes with
the VST3 exporter's MIDI input, after the exporter's first pass, whose first
task checks in Cubase that the host re-reads `IMidiMapping2` after that restart;
the grid does not depend on it.

**Alternatives.** The alternatives were the grid alone, as JUCE built it, with
the pool after the Cubase check, or no learn in VST3 until a later stage.

[History](decision-history.md#d22-controllers-reaching-learn-in-vst3)

### D23. How the processor sends MIDI

**Question and C++ Seam.** How does the processor send MIDI? JUCE's
`processBlock` takes one `MidiBuffer`, read as input and replaced as output.

**Rust.** `Processor::process` takes one `Block`: the channels in place, the
frame count, the events in, and an events-out list whose capacity
`ProcessSetup::max_out_events` fixes at activation (past it, dropped and
counted). A sidechain or transport joins `Block` later without changing the
trait. `PluginInfo::midi_output` declares the output to the host. Parameter
changes still reach the host through the exporter, never this list. No exporter
sends the list's events yet, as none delivers MIDI in (D59, D62).

### D24. Answering MIDI-CI

**Question and C++ Seam.** Who answers MIDI-CI when a plugin must answer for
itself? Not applicable.

**Rust.** A VST3 host speaks MIDI-CI for the plugin, so the VST3 exporter
projects declarations onto VST3's interfaces. Everywhere else a core-tier crate,
`seam-ci`, answers, later: UMP in, UMP out through `Block`, with the C ABI and
CLAP carrying UMP both ways, SysEx7 included.

Each instance takes a random MUID and handles Invalidate MUID (the helper's
fixed MUIDs 64 and 65 are unused constants, and two instances in one host would
collide on them); it drops messages carrying its own MUID, which a loopback
transport echoes back; the audio thread queues SysEx7 to the main thread over a
fixed ring, and replies leave over the next blocks. The parser is Seam's own:
`midi2` 0.11's `ci` module is marked work in progress and holds a Discovery
query only.

### D25. The SysEx manufacturer ID

**Question and C++ Seam.** Who owns the SysEx manufacturer ID that a MIDI-CI
identity needs? (D24) Not applicable.

**Recommendation.** None: a business answer. The working group's helper uses
`0x7D`, which is reserved for non-commercial use. Needed before `seam-ci` ships
an identity.

### D26. Loading the page from a dev server

**Question and C++ Seam.** How does a debug build load the page from a dev
server? (B5) Dev builds read the built page from disk
(`SEAM_WEB_EMBED_PAGES=OFF`, `cmake/seam_web_page.cmake:57-66`), so a page
rebuild needed no C++ recompile but a manual reload; the plugin had no hot
reload (README, "Dev loop").

**Rust.** `SEAM_PAGE_DEV_URL`, read in debug builds only and compiled out of
release builds: set, `seam-webview` loads that URL, so Vite's hot reload works
in the plugin window; unset, it serves the bundled page.

### D27. What runs the generator

**Question and C++ Seam.** What runs the generator? (B3) One shared regeneration
tool for every example, run by a CMake target, an advisory pre-commit staleness
check that never fails a commit, and before every embedded build
(`examples/CMakeLists.txt:66-74`, `.githooks/pre-commit:11-12`,
`cmake/seam_web_example.cmake:156-165`).

**Rust.** An xtask: `cargo xtask gen` writes every generated file, and `cargo
xtask bundle` is to run the generator, the page build and the plugin build in
that order, once a plugin window shows the page. Each output keeps its drift
test for CI. Not built yet: `bundle`.

### D28. Threads that may make a program write

**Question and C++ Seam.** Which threads may make a program write? (P6, P11) Any
but audio; only a UI-thread write told the host (D1).

**Rust.** An edit runs on the UI thread only and joins the parameter's gesture
count (P11), so an edit during a user's drag shares the drag's bracket. A sync
runs on any thread but audio. Its request that the host re-read values is a flag
each exporter reads on its own main thread, not a queued event, since CLAP
allows its rescan only there.

### D29. Gestures on plain values

**Question and C++ Seam.** Do plain values have gestures? (U3, U4, U5, U16, L7)
Yes: a plain value opened gestures as a parameter does, and the recorder tracked
them (`tests/state/undo_recorder_test.cpp:127`).

**Rust.** Yes. A plain value has its own gesture count and token, with no host
to tell, and its record carries the value at the open (D2), so one drag on any
control is one undo step.

### D30. An NRPN data entry with only its high byte

**Question and C++ Seam.** An NRPN data entry sent with only its high byte. (L2)
`msb << 7`, as MIDI 1.0 specifies (a new high byte sets the low byte to 0), so a
sender that never sends the low byte tops out at 0.99225.

**Rust.** Follow MIDI 1.0: a new high byte dispatches the 14-bit value with the
low byte at 0, and a low byte that follows refines it, possibly a block later. A
low byte sent before its high byte since the address changed waits, and the high
byte then dispatches with it (D70). A binding whose capture saw no low byte
scales the high byte as 7-bit instead, so such a sender reaches 1.0. The binding
records which, and the `midiLearn` section saves it from schema 1.

[History](decision-history.md#d30-an-nrpn-data-entry-with-only-its-high-byte)

### D31. What identifies a parameter to hosts

**Question and C++ Seam.** What identifies a parameter to VST3, CLAP and AU
hosts: its declaration index or a hash of its key? (P10, H5) A hash of the key.
JUCE 9.0.0 derives VST3 and AU parameter ids from the parameter id's hash unless
`JUCE_FORCE_USE_LEGACY_PARAM_IDS` is set, and seam-cpp does not set it
(`juce_audio_plugin_client_VST3.cpp:518-527`,
`juce_audio_plugin_client_AU_1.mm:2443-2446`). So P10's "permanent automation
index" held only for formats keyed by index.

**Rust.** A hash of the key, so a later version of a plugin can insert, reorder,
regroup or remove parameters without orphaning automation in VST3, CLAP or AU.
The hash function is fixed forever and pinned by a test of known keys and ids;
VST3 ids stay within 31 bits. The builder refuses, in every build, a key whose
hash collides with another's or falls in the range reserved for the VST3
exporter's own proxy parameters (D22). The declaration index stays the order
hosts list parameters in, and nothing else.

The key was already permanent, since saved state stores values by key. AAX keys
by the key itself. No per-parameter id override for now; one can be added for a
plugin that must match a product already shipped with other ids. The hash and
its refusals live in `seam-params` (D56); each exporter hands the id to its
host.

### D32. When the host hears a gesture begin

**Question and C++ Seam.** When does the host hear that a user's gesture began?
(P6, P11) When it opened: the outermost open forwarded `beginChangeGesture`
(`seam_HostParameter.h:81-87`).

**Rust.** When it opens, so a touch held still reaches the host and Touch and
Latch modes hold the value while P12 holds surfaces. P6's "empty gesture" is a
click that changes nothing, which P5's check before opening rules out. A program
edit and a replay open only on a changing write, so they tell the host then.

### D33. Undo during a user gesture

**Question and C++ Seam.** Undo or redo requested while a user gesture is open?
(U8, U11, U12) Not checked against C++.

**Rust.** Refused with `HistoryError::GestureOpen` until the gesture closes, so
a replay is always one complete host gesture (U11) and the next step starts from
what the replay left (U8).

[History](decision-history.md#d33-undo-during-a-user-gesture)

### D34. A sync while a gesture is open

**Question and C++ Seam.** Does a sync reach surfaces while a gesture is open?
(P6, P12) Not applicable: C++ had no sync intent.

**Rust.** No: it waits with the host-shaped writes and shows at the first poll
after the close. A sync comes from a thread other than the UI's, so it cannot
move what the gesture shows, and showing it mid-drag would yank the drag, which
P12 exists to prevent. After the close it stands unless the drag moved again
(D11).

[History](decision-history.md#d34-a-sync-while-a-gesture-is-open)

### D35. A reserved NRPN block

**Question and C++ Seam.** How does a reserved NRPN block dispatch? (L2, L9)
"Any channel" expanded into 16 dispatch keys, and only the reserved channel's
was dropped (`seam_MidiDispatchTable.h:208-209`).

**Rust.** A mapping whose group and channel could hear a reserved number is
refused, so an any-channel mapping of a number reserved on one channel cannot
exist. Reservations name a product's own protocol numbers, where such a mapping
is a mistake. A saved entry that a later reservation blocks drops at restore
like any bad entry (L9).

[History](decision-history.md#d35-a-reserved-nrpn-block)

### D36. The order one undo tick records

**Question and C++ Seam.** In what order does one undo tick record the changes
it sees? (U5, U14) Synchronous listeners recorded each UI-thread write as it
happened.

**Rust.** In the order they happened. Every parameter gesture record, plain
value commit and list verb takes a number from one sequence on the
`RestoreClock`, under its own lock, and the tick records by it: a gesture at its
close, a plain value's coalesced writes at the newest, a list verb when made.

One undo between two dependent changes, such as an index into a list and the
list, lands on a state that existed, when each value changes on one side of the
other change only: a value written on both sides of it takes the place of its
newest write.

D42 keeps the order across ticks.

[History](decision-history.md#d36-the-order-one-undo-tick-records)

### D37. Whether the history keeps enrolled values alive

**Question and C++ Seam.** Does the history keep enrolled values and lists
alive? (U13) A list forgot itself when destroyed (`seam_UndoList.h:80-90`); a
parameter or scalar had to outlive the recorder or be forgotten
(`seam_UndoRecorder.h:1213`, `:1236`, `:1311`).

**Rust.** No. A tracker holds its value or list by a `Weak` handle, so dropping
the last handle frees it and its queued commits at once, and the history forgets
the enrollment at its next tick. Its steps stay listed and replay as nothing;
elements they hold drop with them.

[History](decision-history.md#d37-whether-the-history-keeps-enrolled-values-alive)

### D38. The origin of a newest write

**Question and C++ Seam.** Do parameters and lists carry the origin of their
newest write? (U10) Not applicable: C++ stopped a write derived from a replay
only while the replay ran synchronously, and never in the deferred case (U10).

**Rust.** Yes, beside the change counter. `ParamSet::read`, `Value::read` and
`List::read` return the count, the value and its origin together, from one load
or one lock, so a deriver never pairs one write's value with another's origin. A
parameter keeps it in one atomic word with its value, so two writers racing on
two threads never leave one's value beside the other's origin. A value derived
from either, recomputed on a later poll, passes the origin on, and a list's
verbs take one through `List::derived`.

A value derived from a parameter takes the value and the origin from one
`ParamSet::read`, which reads both from the one word, so they name the same
write even while another writer races it. A derived change records nothing
(D43). A list verb that records nothing makes the list's earlier steps replay as
nothing, since they index into contents it changed. Before any write the origin
is `Restore`.

[History](decision-history.md#d38-the-origin-of-a-newest-write)

### D39. A load between a replay's check and its write

**Question and C++ Seam.** What stops a load on another thread landing between a
replay's check of the restore clock and its write? (U16, H3) Not checked against
C++.

**Rust.** A lock in the `RestoreClock`. A `Load` takes it for each parameter
write and moves the clock's stored count under it, and each replay step that
writes parameters holds it across its check of the stored count and its write.
So either the replay's write lands before the load's, or the replay sees the
load's write and stops, and the walk clears the history. That holds for a step
recorded after the load began, too. Without it, an undo could store one
pre-load value over a loaded one, and nothing re-applies parameters after a
load (S13). A drag still open when the load lands, a lone write included, is
outside this: its step keeps its pre-drag before, and undoing it writes that
value (U16, D11).

Only parameter steps hold it: a load writes only parameters, and a parameter
write takes no other lock, so it is a leaf lock.

[History](decision-history.md#d39-a-load-between-a-replays-check-and-its-write)

### D40. One undo history per UiParams

**Question and C++ Seam.** How many undo histories may one `UiParams` have? (U1)
Not checked against C++.

**Rust.** One. `UiParams::take_gesture_records` hands out the one drain of
gesture records, `None` while it lives, and `History::new` takes it, so a second
history over the same handle is refused with `HistoryError::HistoryExists`,
which says why. Two would take each other's records, and each could enroll the
same parameter. Records queue only while the drain lives, so none pile up with
no history.

[History](decision-history.md#d40-one-undo-history-per-uiparams)

### D41. A list step replaying against a waiting verb

**Question and C++ Seam.** A list step replays while a verb from another thread
waits for the tick: what then? (U17) Not checked against C++.

**Rust.** The step drops the waiting verb, which then records nothing, and moves
the list's generation, so every step recorded before replays as nothing, as
after a reset. Neither the step's index nor the verb's accounts for the other.

**Alternatives.** The alternatives were a per-step skipped state, or enrolled
lists written only on the UI thread.

[History](decision-history.md#d41-a-list-step-replaying-against-a-waiting-verb)

### D42. Order across ticks

**Question and C++ Seam.** Does a change another thread makes while a tick runs
keep its order? (D36, U14) Not applicable.

**Rust.** Yes. The tick takes a number from the sequence as it starts and
records only changes numbered before it; later ones wait for the next tick, in
order. Each queue is in number order, so the tick takes a prefix of it.

**Alternatives.** The alternative was to say D36 holds only for writes on the UI
thread.

[History](decision-history.md#d42-order-across-ticks)

### D43. A derived change records nothing

**Question and C++ Seam.** Does a change derived from a recorded change record a
step of its own? (U10, D38) Not checked against C++.

**Rust.** No. `Value::set_from` and `List::derived` never record, whatever
origin they pass on: the source's step covers the change, and undoing the source
derives it again, so one user action stays one step. A change that must undo
with its source as one step, a selection clamped after a remove say, is an
ordinary write made in the same handler as its source, inside the source's
transaction; a derivation on a later poll cannot join it.

[History](decision-history.md#d43-a-derived-change-records-nothing)

### D44. A participant that changes while its owner holds it

**Question and C++ Seam.** A participant tells the session it changed from its
own method, while its owner holds it mutably: what then? (S4) The registry
re-captured at once, on the message thread (`ZoomPanel::setZoom` calls
`markDirty`, `session_registry_test.cpp:830-835`).

**Rust.** `Registration::changed` cannot borrow the participant then, so it
marks the registration held up and the next `SessionUi::tick` captures it; a
save shows the change one tick later. A tick captures every participant held up
before it runs an apply pass, and so does a tick made from inside one of their
captures, so a pass that ends a held-up registration keeps its change under its
key.

**Alternatives.** The alternative was to return whether it captured and require
callers to call it after the borrow ends.

[History](decision-history.md#d44-a-participant-that-changes-while-its-owner-holds-it)

### D45. Registrations when a SessionUi drops

**Question and C++ Seam.** What happens to registrations when the `SessionUi`
they were made through drops? (S2) Not applicable: C++ registered with the
registry itself.

**Rust.** They end, as if each `Registration` had dropped: their sections stay
in every save, their keys are free for a later handle, and `connected` turns
false.

**Alternatives.** The alternative was to require the handle to outlive every
registration.

[History](decision-history.md#d45-registrations-when-a-sessionui-drops)

### D46. A derived write and a plain value's open gesture

**Question and C++ Seam.** What does a derived write do to a plain value's open
gesture? (D43, U15, U16) Not checked against C++.

**Rust.** By its origin. A restore or a replay rebases the gesture, which
records from the value it wrote (U16). A write derived from a user's or the
plugin's own change joins the gesture as its newest after-value, so a linked
value echoing the user's drag back keeps the drag. A write derived from a host
or learned change leaves the gesture's before and after alone (U15); the
baseline still follows it.

[History](decision-history.md#d46-a-derived-write-and-a-plain-values-open-gesture)

### D47. When a lone NRPN high byte settles as coarse

**Question and C++ Seam.** When does a lone NRPN high byte settle as coarse?
(L3, D30) Not applicable: C++ learned `msb << 7` at once.

**Rust.** When a learn poll finds the same unsettled capture an earlier poll
found, and a whole audio block after the one that staged it has ended: the low
byte had a UI tick and a block to arrive and did not, and no newer message came.
The audio thread counts its blocks as they begin and end, so a poll can tell
whether a block ran since an earlier one; while the host runs no blocks, the
capture waits. A second high byte with no low byte between still settles it at
once on the audio thread.

[History](decision-history.md#d47-when-a-lone-nrpn-high-byte-settles-as-coarse)

### D48. A plain Value as a learn Target

**Question and C++ Seam.** How does a plain `Value` serve as a learn `Target`?
(L7, D29, D33) Not applicable: C++ had no "the user owns it" rule (L7).

**Recommendation.** Open. `Value` reports only whether some gesture is open, so
`Target::held` cannot leave out learn's own burst: counting every gesture splits
one sweep into many bursts, leaving learn's out lets MIDI write through the
user's drag, and a grab inside an open burst merges the sweep into the user's
undo step. Undo's D33 also refuses while a steady MIDI stream keeps the burst
open.

The recommendation: tag a `Value` gesture as the user's or learn's, with its own
before and after, and have D33 wait only for the user's. The cheaper
`Value::gestures() -> u32` fixes `held` alone. Needed before a plugin learns
into a plain value.

[History](decision-history.md#d48-a-plain-value-as-a-learn-target)

### D49. A participant writing a parameter during apply

**Question and C++ Seam.** May a session participant write a parameter through
the UI handle while it applies? (U16, P6) Not checked against C++.

**Rust.** No: it writes through `ProgramParams::sync`, which records nothing and
tells the host no gesture. A user or edit write through `UiParams` while the
applying mark is up fails a debug assertion.

**Alternatives.** The alternative was to make such writes act as a sync under
the mark.

[History](decision-history.md#d49-a-participant-writing-a-parameter-during-apply)

### D50. Learn's audio side per activation

**Question and C++ Seam.** How does each activation's processor get learn's
audio side? (L5) Not applicable: C++ ran learn inside the processor.

**Rust.** `Learn::replace_audio()` makes a fresh one that reads the current
mappings; the one handed out before keeps its last table and sees no later edit.
A VST3 host reactivates at every sample-rate or block-size change, and
`Learn::new` made the only one.

**Alternatives.** The alternative was a `Plugin::deactivate` that hands the old
processor back.

[History](decision-history.md#d50-learns-audio-side-per-activation)

### D51. Whether a plugin must be Send

**Question and C++ Seam.** Must a plugin be `Send`? (U5, S6, L7, H1) Not
applicable: the C++ Seam had no plugin trait.

**Rust.** No. The exporter calls the plugin and drops it on its main thread
only, so a plugin owns its `Parts`, its undo `History` and its watches, none of
which is `Send` (D91). It creates it there too.

Only a plugin that is `Send` leaves the main thread, through the C interface,
which binds it to no thread (D62). For the instance's whole life, the interface
lends the plugin to one main-thread call at a time, on whichever thread the call
arrives. A call that meets another waits its turn, up to a bound, and a tick
that meets one skips. The plugin drops on the thread that destroys it.

Seam builds the plugin's `Arc<SavedState>`, which is `Send + Sync`, with its
parts, and the exporter keeps it (D91). The host's save and load calls, on
whatever thread they arrive, go through it and never touch the plugin, and the
set the host is shown is the one it saves (`SavedState::set`, D52). A plugin
that is `Send` keeps its parameters and drops its parts. Both exporters assert
this to their hosts with
`unsafe` in the boundary module they share (`MainThread`).

**Alternatives.** The alternatives were a `Send` plugin beside a separate
main-thread type that holds those handles, or a wrapper that checks the thread
at run time.

[History](decision-history.md#d51-whether-a-plugin-must-be-send)

### D52. Who makes the RestoreClock

**Question and C++ Seam.** Who makes the `RestoreClock`? (D19, D20, D39) Not
applicable.

**Rust.** The `ParamSet`: `ParamSetBuilder::build` makes one, and nothing else
can. `ParamSet::take_ui`, `ProgramParams::load`, `Value::new`, `List::new` and
`Session::new` take it from the set, undo's `History::new` from the `UiParams`,
and `SavedState::new` and `Learn::attach` panic when the session was made from
another set, so every history, value, list, session and load of one plugin
shares one clock by construction. Seam builds a plugin's set, session and
saved state together (D91). Two sessions on one set are not caught: the saved
block saves only the one it was given.

**Alternatives.** The alternatives were a check the plugin calls, or a builder
in `seam-host` that wires the pieces.

[History](decision-history.md#d52-who-makes-the-restoreclock)

### D53. What ticks a plugin with no editor open

**Question and C++ Seam.** What ticks a plugin with no editor open? (U5, S6, L7,
D47) Learn ran its own 60 Hz JUCE timer on the message thread, editor open or
not, while armed, writing targets or holding retired tables
(`seam_MidiLearnEngine.h:125`, `:916-922`). The reconcile pump lived with the
editor and ran only while one was open, at 30 Hz idle and 90 Hz in a burst
(`seam_ReconcilePump.h:20-35`).

**Rust.** One main-thread timer per instance, at 60 Hz for the instance's life,
editor open or not, ticks the plugin's parts and then calls `Plugin::idle`
(D91); the editor never ticks. VST3 gives a
component no idle call without an editor, so its exporter runs its own, from
`initialize` to release, on the thread `initialize` binds (D59): a message-only
window's timer on Windows, about 64 Hz, and a Core Foundation run loop timer in
the run loop's common modes on macOS, at 60 Hz, so a host's modal loop still
ticks it.

On Linux a VST3 host offers its run loop only through the editor's frame (JUCE
runs its own message thread instead), so the VST3 exporter refuses to build
there (D74). The JUCE adapter ticks from a `juce::Timer` on JUCE's message
thread; CLAP's exporter is to use the host's timer-support extension. 60 Hz
keeps learn's burst of 15 idle ticks at 250 ms, as in C++, and D47's wait for a
low byte at one tick, about 17 ms, or a whole block when blocks run longer than
a tick. Still to settle: a CLAP host may lack timer support.

A release on another thread invalidates the macOS timer from that thread, which
rests on Core Foundation's source, where invalidation takes the run loop's lock.

Without a tick, a host load's mappings never reach the audio thread and undo's
commits pile up.

**Alternatives.** The alternatives were 30 Hz, or the editor's frame loop while
it is open and a timer only while it is closed.

[History](decision-history.md#d53-what-ticks-a-plugin-with-no-editor-open)

### D54. Detaching a learn target

**Question and C++ Seam.** What does detaching a learn target do to its mapping?
(L8, L9) Erased it (`seam_MidiLearnEngine.h:289`).

**Rust.** Holds it. `Learn::detach` ends an arm on the target, closes its open
burst once and drops the target, and its mapping becomes a held entry under its
key, as a restore holds one for a target not registered yet (L9): saved back
unchanged, and applied when that key registers again, under the checks a restore
makes: a table full by then drops it, as it drops a restored entry. `unmap` is
what removes a mapping, so a target dropped for good is unmapped first.

An editor that registers its own targets when it opens and detaches them when it
closes would otherwise lose their mappings at the next save.

**Alternatives.** The alternatives were to erase, as C++ did, or a second verb
that erases.

[History](decision-history.md#d54-detaching-a-learn-target)

### D55. A participant whose apply panics

**Question and C++ Seam.** What happens to a participant whose apply panics?
(S7, D14) A throwing apply hit a debug assertion and kept the raw section
(`seam_SessionRegistry.h:449-459`); the participant stayed registered, and the
pass marked nothing that would stop a later capture.

**Rust.** Catch and hold. The session contains the panic as it does a failed
apply: the pass continues, nothing reaches the host, and the raw section stays
the saved one. The participant stays registered, but its captures are ignored,
`Registration::changed` included, until a later apply succeeds, at the next
restore or one without its section (D6), so a state the panic left half-built is
never saved. A failed apply, which leaves the participant as it was (D14), does
not hold it.

Needs `panic = "unwind"`; with `abort` a panic ends the process.

**Alternatives.** The alternatives were to keep capturing after a panic, which
can save a half-built state over the raw section, or to end the registration,
which drops the participant from saves and restores until it registers again.

[History](decision-history.md#d55-a-participant-whose-apply-panics)

### D56. The hash that names a parameter

**Question and C++ Seam.** Which hash names a parameter to hosts, and which ids
does D22 reserve? (D31, P10) JUCE 9.0.0 hashed the parameter id (D31).

**Rust.** Rust does not match it (D5). 32-bit FNV-1a over the key's UTF-8 bytes
with the top bit cleared, so an id fits VST3's 31 bits and is the same number in
VST3 and CLAP; through the JUCE adapter, AU and AAX derive theirs from the key
as JUCE does. The builder refuses a key whose id another key already has, or
that falls in `0x7FFF_0000` to `0x7FFF_FFFF`, the 65,536 ids D22's proxies take:
the grid's 2,080 and the MIDI 2.0 pool, with room. A test pins the ids of known
keys.

**Alternatives.** The alternatives were another fixed hash, or a 64-bit hash
folded to 31 bits, which collides no less often at 31 bits.

[History](decision-history.md#d56-the-hash-that-names-a-parameter)

### D57. The lattice of a stepped range

**Question and C++ Seam.** From what does a stepped range lay out its lattice?
(P2, P4) In `float`: `start + interval * floor((v - start) / interval + 0.5)`
(`juce_NormalisableRange.h:194`), which puts 0 off zero in about 29% of decimal
ranges.

**Rust.** From the decimals the range was written with, in whole numbers of its
finest place. The start and the interval are each read as the shortest decimal,
up to nine places, that reads back as the same `f32`, and scaled to whole
numbers of the finer one's last place; point k is `(s + k * t) / 10^p`, then
rounded to `f32`. So every point of a decimal range, 0 dB included, lands on the
nearest `f32` to its decimal, which a property test checks.

A range whose start or step needs more than nine places, or whose ends are too
large to count exactly in `f64`, steps by its `f32`s. The step count a host is
told reads the same lattice. `Range::new` lays the lattice out once, when the
range is made, so a write snaps with no decimal work on the audio thread.

The JUCE build in the comparison snaps its ranges the same way, through
`NormalisableRange`'s snap function.

**Alternatives.** The alternative was JUCE's float formula, which a plain JUCE
build matches with no custom code.

[History](decision-history.md#d57-the-lattice-of-a-stepped-range)

### D58. What the plugin trait tells every host

**Question and C++ Seam.** What does the plugin trait tell every host, beyond
its parameters and state? (H5) Not applicable: JUCE's `AudioProcessor` answered
each format.

**Rust.** `PluginInfo` gains the vendor's web and contact addresses and a
one-line description (any may be empty), `Feature`s finer than the category
(each exporter names them in its format's words: VST3's `Fx\|EQ`), and the main
bus `Layout`s the plugin runs with, the first by default.

`Plugin` gains `latency` and `tail`, read on the main thread (the exporter reads
both at creation and at each activation, deactivation and tick, answers the host
from that on any thread, and tells the host when the latency moves, an
activation's included), and `deactivate`, which takes the processor back on the
main thread so what it holds is never freed on the audio thread; only a host
that releases an instance on another thread gets it dropped there instead (D59).

`Processor::process` returns a `Status`, the process status a CLAP exporter
is to read once one is built (D68); no built exporter acts on it, and VST3
ignores it. What every exporter does without being asked: a declared bypass
parameter is the host's bypass and no exporter adds a hidden one (JUCE's VST3
wrapper did); one program, none offered to choose; 32-bit float only. Deferred:
whether the host is rendering offline or in real time. `ProcessSetup` has no
field for it and VST3's `processMode` is ignored; it is settled with CLAP and
the C interface.

**Alternatives.** The alternatives were format-specific category strings in
`PluginInfo`, or a hidden bypass parameter as JUCE adds.

[History](decision-history.md#d58-what-the-plugin-trait-tells-every-host)

### D59. The shape of the VST3 exporter

**Question and C++ Seam.** What shape does the VST3 exporter give a plugin?
(D51, D53, P7, C2) JUCE's wrapper made a separate component and controller that
kept their own parameter copies in step.

**Rust.** One object that is both the audio processor and its own edit
controller, so the host, the processor and the plugin share one parameter set.
JUCE's hosting finds the controller on the component
(`juce_VST3PluginFormatImpl.h`); the SDK's own `PlugProvider` is not in the
local tree, so that half is unchecked.

The component is a thin shell over an exporter core it shares with the C
interface (`seam_host`'s engine), so each rule of the plugin's life below is
written once.

The plugin is built at `initialize`, on the thread that calls it, which becomes
the instance's main thread, since the SDK does not say where a host creates a
component; until then a call answers `kNotInitialized`, or what it answers with
no plugin (no parameters, no latency, the first declared layout). Main-thread
state is reached only on that thread, and a call that must run there and arrives
elsewhere is refused and counted.

This refines D51: the plugin still need not be `Send`, and the exporter checks
the thread at run time rather than ask it to be. The bus layout, latency and
tail are answered on any thread, from what the main thread last read. An
activation while active builds a fresh processor (D73). A release on another
thread leaks the plugin and everything a tick can reach rather than drop them
off their thread, and its processor drops there, inside the panic guard, so a
panic in its drop is caught there.

That release also stops the leaked timer from ticking: on Windows it pins the
module and detaches the timer's window from its tick, and the window then stops
the timer at its next message. The pin stays, since the host's message loop
could otherwise call into a module it has unloaded. On macOS the release
invalidates the run loop timer.

A release from a host callback inside a tick deactivates, makes no further call
to the host and leaks, and a tick that a nested message loop delivers from
inside a host callback does nothing, so a release later in the outer tick still
leaks. The processor sits in a slot the audio thread takes with one
compare-and-swap that never waits.

The host's automation reaches the core as queues of points, each at its sample
offset: the core carries every point it has room for (D72), in the host's order,
and applies each queue's last point before the block, as JUCE's wrapper does, so
the processor still sees one value per parameter per block. `process` and
`setProcessing` run under the audio-thread mark (D67). A host write through the
controller lands at once while the host is not processing.

While it is, from `setProcessing(true)` to `setProcessing(false)` or
deactivation, the write is ignored, since the host sends the same value through
`process` in order with its automation. JUCE's wrapper applies it at once unless
the transport plays, so a host that reads the value straight back sees the new
one there and the old one here until its next block. A block longer than the
setup promised plays in pieces.

Edits reach the host right after each tick (C2), and so does a latency that
moved, an activation's included, as `kLatencyChanged`. D53's timer is a
message-only window per instance on Windows. It asks for 15 ms, which the system
clock's 15.6 ms tick gives: about 64 Hz, so learn's burst of 15 ticks lasts
about 234 ms, not D53's 250. On macOS it is a run loop timer at 60 Hz, and for
any other platform the exporter does not build (D74). No MIDI events are
delivered yet.

**Alternatives.** The alternative was a separate controller class, as JUCE built
it, which needs a second copy of every value kept in step through the host; for
the main thread, the thread that creates the instance.

[History](decision-history.md#d59-the-shape-of-the-vst3-exporter)

### D60. A panic inside a host's call

**Question and C++ Seam.** What happens when plugin code panics inside a host's
call? (P9) Not applicable: C++ had no panics, and an exception into the host was
undefined.

**Rust.** It is caught at every entry point that can panic and never unwinds
into the host; the others only copy constants or touch atomics. A panic in the
processor silences the instance's output until the next activation, which builds
a fresh processor: it is caught while the block still holds the processor's
slot, and the mark stays with that processor, so an activation racing the block
installs one that starts clean. A panic elsewhere in the block, in the
automation walk say, silences that block alone.

Any other call answers the host with an error, or a neutral value where the call
returns one. Each is counted for its instance, on whatever thread it happens,
and for the process (`seam_host::boundary_faults`), with a call refused for its
thread, and reported as D75 says; each test host checks its instance's count
unchanged when it is dropped, less the faults a test provokes and declares.
Needs `panic = "unwind"`, which `export_vst3!` and `export_cabi!` check at build
time.

**Alternatives.** The alternatives were to keep running a processor that
panicked, or to answer the host's `process` with an error, which some hosts meet
by disabling the plugin.

[History](decision-history.md#d60-a-panic-inside-a-hosts-call)

### D61. Who flushes denormals

**Question and C++ Seam.** Who flushes denormals to zero? (the comparison's
fairness row) Each JUCE plugin's own `processBlock`, through
`ScopedNoDenormals`, which JUCE's project template puts there; JUCE's VST3
wrapper sets nothing.

**Rust.** Every exporter, around every `process` call: FTZ and DAZ on x86, FZ on
AArch64, restoring the host's setting after, so no plugin has to. The flush
starts after the host's parameter changes are read, as JUCE's wrapper reads them
outside the plugin's `processBlock`. The JUCE build in the comparison starts its
`processBlock` with `ScopedNoDenormals`, as JUCE's template does, so both sides
flush; the bench's parity check still matches bit for bit with it.

**Alternatives.** The alternative was to leave it to each plugin, as JUCE does,
which in Rust would put inline assembly in every plugin.

[History](decision-history.md#d61-who-flushes-denormals)

### D62. The C interface for the JUCE adapter

**Question and C++ Seam.** What shape is the C interface the JUCE adapter is
driven by? (H5, D51, D60) Not applicable: the C++ Seam was a JUCE module.

**Rust.** One exported symbol, `seam_plugin`, answering a table of function
pointers, as CLAP's entry point does: the plugin's layouts and every call into
an instance, each with the thread it may run on. Text is UTF-8 with a length,
lent from the declarations; a text call with no room for the text answers its
length, so a caller can size a buffer.

**Parameters and callbacks.** Parameters are reached by their index in the order
hosts list them (P10); a host write (P7) may come from any thread, the audio
thread included, as JUCE's wrappers deliver automation through it, and it always
lands, since the C interface has no processing state to hold it off by. The
host's callbacks (begin, perform and end an edit, restart) run only inside
`idle` and `destroy`, on the thread that made that call, and may not unwind. The
instance is a thin shell over the exporter core the VST3 exporter shares (D59).

**Threads.** Which thread is main follows the plugin's type, chosen where
`export_cabi!` names it. A plugin that is `Send` is bound to no thread, ever:
every main-thread call, `idle` and `destroy` included, takes the plugin on loan
for its length, on whichever thread it arrives, for the instance's whole life.
An `idle` that arrives on another thread while a call has it skips, counting and
reporting nothing, and the next tick runs as usual.

**Waiting.** Any other main-thread call but `destroy` waits for the loan, up to
50 ms (`LOAN_WAIT`) from its first refusal, then takes it; past the bound it is
refused and counted as a fault. The bound is what keeps a host that holds a lock
of its own across both threads from deadlocking. The wait spins briefly, yields,
then sleeps a millisecond at a time, and each try is the loan's own
compare-and-swap. A call made from inside one on the same thread, as a host
callback inside `idle` makes, shares its loan.

**Overlap.** The instance keeps the calls apart itself, since its caller does
not: JUCE's VST3 wrapper calls `prepareToPlay` from `setActive` on the host's
thread holding no lock, while the adapter's timer ticks on the message thread,
so the two meet only when they overlap: an activation that meets a tick waits
for it, and a tick that meets an activation skips.

**Plugins that are not `Send`.** A plugin that is not `Send` keeps the thread
that creates it, and a main-thread call from elsewhere is refused and counted;
JUCE's VST3 wrapper creates the plugin on the thread the host creates it on.

**Teardown.** A `destroy` takes the plugin without waiting. A `destroy` that
cannot is counted as a fault and leaks the instance. Such a `destroy` comes on
another thread for a plugin that is not `Send`, or while another thread's call
holds a plugin that is. In the second case the other call goes on using the
instance until it returns, even after it lets the plugin go. The leaking
`destroy` drops only the processor, on its own thread, inside the panic guard.
The leaked instance's tick does nothing more, but a tick already running
finishes, and may make a callback after `destroy` returned, which only a host
that breaks the rule meets.

**Teardown from a callback.** A destroy from a host callback inside `idle` leaks
it deactivated, and the tick makes no further callback, since the host may be
gone. An `idle` called from inside one of those callbacks, as a nested message
loop delivers JUCE's timer, does nothing.

**AAX.** JUCE's AAX wrapper calls `prepareToPlay` on the render thread when a
longer block arrives: a plugin that is `Send` takes it there, waiting there for
a tick it meets, and one that is not refuses it while the processor already
built plays the block in pieces. Open: that wait lands on the audio thread,
though no main-thread call is the audio thread's by contract.

**Linux.** On Linux a VST3 host with no run loop of its own leaves JUCE's
message thread apart from the creating one: a plugin that is `Send` is ticked
there, and one that is not has every tick refused and reported.

**Other rules.** `set` and `reset` run under the audio-thread mark (D67). The
header and the library are built from one commit, so the table carries a version
the adapter checks and no other compatibility promise. Panics are caught at
every entry point (D60), and the processor flushes denormals inside `process`
(D61).

**MIDI.** No MIDI passes yet; it arrives as UMP both ways with the exporters'
MIDI input (C5).

**Alternatives.** The alternatives were one exported function per call, or a
table filled in by the adapter. For the main thread, they were:

- the creating thread, for every plugin;
- the first main-thread call of any kind, under which a `prepareToPlay` off the
  message thread before the first tick took that thread, and every later tick
  was refused;
- the thread of the first `idle`;
- an adapter that binds the thread before it activates;
- an adapter that keeps its activations, deactivations, ticks and destroy apart
  with one mutex, which leaves the soundness of the Rust side to C++.

[History](decision-history.md#d62-the-c-interface-for-the-juce-adapter)

### D63. Keeping the C header true

**Question and C++ Seam.** How is the C header kept true to the Rust side? (D27)
Not applicable.

**Rust.** Seam's own generator writes `crates/host/include/seam.h` from the Rust
declarations. Each type spells itself in C, so every field's C declaration
follows its Rust type and a change of type changes the header, which the drift
gate catches; only parameter names are written by hand, and their count is
checked against each function's. The header then asserts every struct's size and
every field's offset as Rust laid them out, so a C++ build against a header
whose layout no longer matches fails to compile.

It needs C11 or C++11, and refuses a target of another pointer width; CI
compiles it as both, for x86_64 and AArch64, so the assertions are checked on
every change. C's `char` crosses as `CChar`, a transparent newtype over
`c_char`, which the header spells `char`, so the interface builds where `char`
is unsigned, as on AArch64 Linux and Android. The header also states the pointer
contract the plugin relies on and does not check beyond null. `cargo xtask gen`
commits it.

**Alternatives.** The alternative was cbindgen: a new dependency, which reads
the source rather than the types and cannot see an entry point a macro makes.

[History](decision-history.md#d63-keeping-the-c-header-true)

### D64. How the JUCE adapter builds a plugin

**Question and C++ Seam.** How does the JUCE adapter build a plugin? (H5, D18)
Not applicable.

**Rust.** It links the plugin crate as a static library built with only its
`cabi` feature, since JUCE's VST3 wrapper defines `GetPluginFactory` itself: a
plugin crate exports with `export_vst3!` behind a default `vst3` feature and
`export_cabi!` behind `cabi`.

JUCE's build-time facts come from a `seam-juce.cmake` that `cargo xtask gen`
writes per plugin from `PluginInfo`: names, the AU codes (D86; from which JUCE
also derives its VST3 class id, so the adapter's build is a different plugin
from Seam's native one), the VST3 categories, MIDI. Writing it refuses keys
whose JUCE ids collide, with each other or with JUCE's own bypass and program
parameters.

`seam_add_juce_plugin` in `adapters/juce/SeamJuce.cmake` reads them, runs cargo
on the plugin's manifest (Seam's workspace by default, or a product's own) and
adds the adapter's one processor class, which holds no product code. The plugin
builds against the seam-host the adapter comes with, whose committed `seam.h` it
uses. The Rust side is built with cargo's release profile and takes the C
runtime the JUCE target links, so Debug and Release builds both work.

On macOS and iOS it is built once per architecture in `CMAKE_OSX_ARCHITECTURES`
(the host's own when that is empty), each with `cargo --target` and the
deployment target when one is set, and the archives are joined with `lipo`, so a
universal plugin links a universal library; each architecture's Rust target must
be installed, and the iOS simulator's targets are not handled.

**Alternatives.** The alternative was loading the plugin's `cdylib` at run time,
which makes every format two binaries.

[History](decision-history.md#d64-how-the-juce-adapter-builds-a-plugin)

### D65. What a host sees through JUCE

**Question and C++ Seam.** What does a host see of a plugin through JUCE? (D31,
D53, D58, C4) What JUCE makes of any plugin.

**Rust.** What JUCE 9 makes of it, as of the JUCE EQ; checked with VST3 on
Windows. Parameter ids are JUCE's hash of the key (its Studio One-compatible
form, `seam_host::cabi::juce_param_id`), the ids the JUCE EQ has, not D31's:
JUCE offers no way to set them, and its legacy ids are indices. JUCE's VST3
wrapper adds its own bypass parameter when the plugin declares none, so the EQ
lists 26 there, as A does. For a plugin that takes MIDI input it also adds an
id for each MIDI controller, so `juce_settings` refuses a key whose id lands
there.

A host's bypass through it reaches the adapter's `processBlockBypassed`, which
delays the input by the latency the last `prepareToPlay` reported, from a delay
line cleared as each bypass starts, where JUCE's own passes it through undelayed
and a host that compensates for the latency plays it early; a latency that grows
later is delayed by at most the reported one until the next `prepareToPlay`.

JUCE hands the host's 0..1 values over as `float`, so a host write reaches Seam
rounded to `f32` where Seam's own exporters pass the host's `double`, and its
wrapper drops one equal to the current value before it reaches Seam. A
controller write lands as JUCE's wrapper applies it: at once, unless the
transport plays (D59).

An edit the plugin makes reaches JUCE as a notification, not a write; JUCE's
wrapper sets its own copy and, while the transport is stopped, writes it back,
which stores what the store already holds unless a host write landed since. JUCE
has no "re-read every value" for a plugin with one program, so a sync (D1)
reaches the host as a value notification for each parameter that moved, which
JUCE sends on as an edit outside any gesture; a host in latch mode may record
those, to check with AU in Logic and Pro Tools (C4).

The host's state is Seam's block followed by JUCE's own trailer, so a state
saved through JUCE does not load into Seam's native build (H2). A text that
means no value answers the current one, since JUCE has no way to say none. A
host's text reaches the plugin's parser as the UTF-8 JUCE holds, read in place
up to its first NUL. The one program a plugin declares (H5) has the plugin's
name, as the Steinberg validator requires of a program list. The adapter
clears each block's MIDI buffer before it processes, since JUCE's wrappers send
on what the buffer holds when a plugin produces MIDI. The adapter ticks
the plugin from a `juce::Timer` at 60 Hz, editor open or not (D53), on JUCE's
message thread, where a plugin that is `Send` is lent to each tick as on any
other thread (D62); its constructor does not assert that it runs there.

Through JUCE's VST3 hosting (`bench/scripts/hosted.py`), the adapter's EQ plays
the parity script bit for bit against the Rust EQ driven directly, at 44.1, 48
and 96 kHz. CI builds the EQ through the adapter as VST3 and AU on macOS, for
arm64 and x86_64, without loading it.

[History](decision-history.md#d65-what-a-host-sees-through-juce)

### D66. Bytes after the block, and a failed load

**Question and C++ Seam.** What does a load do with bytes after the block, and
is the host told that a load failed? (H2) Read a block with bytes after the
closing brace (H2); JUCE's `setStateInformation` answers a host nothing.

**Rust.** Trailing NUL bytes, which a host that stores blocks in fixed-size
chunks pads them with, are trimmed and the block loads. Any other byte after the
object, a following chunk say, makes the block unrecognised, so it changes
nothing. JSON whitespace around the object is part of the JSON text and loads. A
load that fails, for that or any other reason H2 lists, is answered as failed:
VST3's `setState` answers `kResultFalse` and the C interface's `load` false.
Through JUCE, whose `setStateInformation` answers nothing, the adapter cannot
tell its host. Where the answer is only a refusal, the first refused load of an
instance also writes one line with the reason (the error's message, then each
source's) where faults go; later ones write nothing, and a refused load is not a
fault, so no count moves.

**Alternatives.** The alternatives were to ignore trailing bytes and load the
first value, or to refuse any, NUL padding included.

[History](decision-history.md#d66-bytes-after-the-block-and-a-failed-load)

### D67. Checking the audio-thread locking rule

**Question and C++ Seam.** How are the rules checked that keep locking types off
the audio thread? (P6, D39) Not checked against C++.

**Rust.** By a debug-build mark, `seam_params::audio_thread`: a per-thread depth
an exporter raises around everything its `process` call runs, and around the
other calls a host may make on the audio thread (the C interface's `set` and
`reset`, VST3's `setProcessing`). Every call that locks asserts the mark is
absent: `Value` and `List` calls, `Session` calls, `Load::restore` and the load
hold an undo replay takes; so do the drops of `AudioParams`, `LearnAudio`,
`HostEventDrain` and the typed parameters `Float`, `Bool` and `Choice`, which
must not free on the audio thread. A typed parameter's clone is allowed there.

A thread already unwinding is not checked, so a drop during a panic cannot turn
it into an abort. A release build has neither the mark nor the checks. VST3's
`setParamNormalized` is not marked, since the SDK puts it on the UI thread. The
types stay `Send` and `Sync`, so the compiler cannot keep them off the audio
thread, and the mark catches only what a test or a debug build runs.

**Alternatives.** The alternative was a typed audio-side handle that makes the
rules compile-time.

[History](decision-history.md#d67-checking-the-audio-thread-locking-rule)

### D68. Audit items that wait for their code

**Question and C++ Seam.** Which of the 2026-11-05 audits' items wait until
their code exists? Not applicable.

**Rust.** Those whose code has not started: the CLAP exporter, `seam-webview`
(D16), the native side of the page protocol (no Rust code handles a page message
yet, B1), automation smoothing and splitting, and semver checks. Findings that
rest on them stay open until then. The core already keeps each automation
point's sample position (D59), so smoothing or splitting can build on it. The
derive that declares each parameter once was on this list until it was built
(D93).

The first audit's plan made smoothing the default and splitting opt-in with a
32-sample floor; the second recommended an opt-in smoothing helper, and
splitting only once a bench scenario sends several points per queue; which holds
is decided with the code.

**Alternatives.** The alternative was to build them on the audit branch.

[History](decision-history.md#d68-audit-items-that-wait-for-their-code)

### D69. Where a continuous range is checked

**Question and C++ Seam.** Where is a continuous range checked, and how does a
declaration that is a constant build one? (P2, P10, D57) JUCE's
`NormalisableRange` asserted its invariants in debug builds only.

**Rust.** When it is made. `Range` keeps its fields private, and `Range::new`, a
`const fn`, returns a `RangeError` when an end is not finite, the start is not
below the end, the interval is negative or not finite, or the skew centre is not
strictly inside; the builder does not check ranges. `Kind::continuous`, a public
`const fn`, panics on what `Range::new` refuses, so a wrong range in a
declaration that is a constant fails the build.

A typed declaration writes its range as one, `FloatDecl::new(key, name,
-60.0..=12.0, default)` with `with_step` and `with_skew_centre`, so it holds
no `Range` until it is checked, and `Range::new` checks it in two places.
`FloatDecl::checked`, a `const fn`, panics on a refused range, which in a
declaration that is a constant fails the build. `Declarer::add` refuses it
with `DeclError::RangeRefused`, naming the key. Both also refuse a default
outside the range (D90).

**Alternatives.** The alternatives were public fields checked by the builder in
every build, or `Range::new` alone, each constant declaration matching its
result by hand.

[History](decision-history.md#d69-where-a-continuous-range-is-checked)

### D70. Learn's reading of MIDI 1.0 pairs

**Question and C++ Seam.** How does learn read MIDI 1.0's paired controllers and
parameter numbers where MIDI 1.0 leaves room? (L2, L3, D30) Assembled NRPNs from
controllers 99, 98, 6 and 38 and treated every other controller as plain (L2);
the rest was not checked against C++.

**Rust.** Per group and channel. The two address bytes pair in either order, as
MIDI 1.0 allows and JUCE's `MidiRPNGenerator` sends them (98 first): a byte
completes the address with the other one when that one arrived since the last
data entry, RPN address or completed address.

A 99 with nothing to pair with forgets the 98 held, so a lone 99 never completes
from a stale low byte; a 98 alone changes the low byte under the 99 held; 127
with 127 is the null number and selects nothing, so stray data entry drives no
mapping. An RPN address (101, 100) selects RPN and leaves the NRPN bytes held,
so a later 98 alone selects that NRPN again.

Reset All Controllers (121) forgets both address bytes and the selection, where
its recommended response (RP-015) sets the number to null, and still passes on
as controller 121. A data entry low byte sent before its high byte waits, and
the high byte takes it; an address change drops it.

A controller 32 to 63 that comes right after the controller 32 below it on the
same group and channel, with no other capture between, learns that one, the
pair's high byte, as a 7-bit controller, since the low byte alone binds half a
value that wraps as the knob turns. The first poll after the high byte commits
it, so the pairing window ends there rather than after a number of blocks,
which a DIN pair straddles at small buffers.

A controller 32 to 63 with no high byte just before it waits as a lone NRPN
high byte does (D47), so the high byte that the knob's next move sends first
wins; otherwise it learns itself, which leaves a sender that sends only low
bytes open. `Learn::poll` (`crates/learn/src/mappings.rs`) states the rule and
the open question in full.

A mapping then reads only the high byte: there is no 14-bit controller source,
and a learned pair plays at 7 bits. Data increment and decrement (96, 97) belong
to the parser, never learned or bound, and end the stretch in which address
bytes pair, as data entry does.

**Alternatives.** The alternatives were a 14-bit controller source that reads
both bytes, as an NRPN's data entry does; RPN selection that clears the NRPN
bytes; and a Reset All Controllers that only deselects, keeping the bytes.

[History](decision-history.md#d70-learns-reading-of-midi-10-pairs)

### D71. Learn after an unreadable midiLearn section

**Question and C++ Seam.** What does learn route after a restore whose
`midiLearn` section it cannot read? (L9, D14, H2) Cleared the table (L9).

**Rust.** Nothing from before it. Learn ends the arm, clears every mapping and
held entry and publishes the empty table, then closes open bursts (D82) and
fails the apply, so the session keeps the section for a build that reads it
(D14), and the same block always gives the same routing (H2). Learn is the one
participant that changes its state before failing an apply.

**Alternatives.** The alternative was to keep the routing, as D14 reads for any
other participant.

[History](decision-history.md#d71-learn-after-an-unreadable-midilearn-section)

### D72. How much a host may ask

**Question and C++ Seam.** How much may a host ask of an instance? (H2, D59,
D62) Not checked against C++.

**Rust.** Fixed limits, so no claim of a host's can exhaust memory or hold the
audio thread. A processor is built for at most 65,536 frames a block
(`MAX_FRAMES`, `SEAM_MAX_FRAMES` in the header; about 1.4 seconds at 48 kHz),
since its buffers are allocated up front. A setup that promises more is built
for that many in both exporters, and a longer block plays in pieces. A saved
block is at most 64 MiB (`MAX_STATE_BYTES`): a longer one is refused as
unrecognised, and a VST3 stream is read no further once past it.

A block's automation has room for 8 points per parameter on average, shared by
its queues: each queue's last point is read first and lands. A queue whose last
point cannot be read still counts as taken, and none of its other points is read
or counted. Earlier points are read while the room lasts, a read that fails
using it up too, and every point neither carried nor a failed read is counted as
dropped.

The walk takes at most as many queues as the plugin has parameters, and passes
over at most 4,096 others: a missing or empty queue, an id the plugin does not
declare, such as a hidden proxy, or a second queue for a parameter already
taken. Each count has its own bound, so a host that lists unknown ids first
cannot use up the room the plugin's own queues need. The 4,096 bound is above
D22's grid of 2,080 proxies, with room for its MIDI 2.0 pool, so a block that
moves every proxy still reaches the plugin's own queues.

A second queue for a parameter is passed over unread, so the first queue wins
wherever the repeat comes, even one whose last point cannot be read. A block
that runs while another holds the room, as when a host calls `process` from two
threads at once, cannot tell a repeat apart. There each queue writes its last
point at once and counts toward the parameters, so a repeat's value stands and
the walk ends a queue sooner for each.

MIDI keeps 2,048 events each way per block, as before.

**Alternatives.** The alternatives were other sizes, 256 MiB for the state say,
room per queue rather than shared, or a setup past the frame limit refused
rather than built for the limit.

[History](decision-history.md#d72-how-much-a-host-may-ask)

### D73. What an activation makes of an open setup

**Question and C++ Seam.** What does an activation make of what the host's setup
leaves open? (D58, D59, D62) Not checked against C++.

**Rust.** An activation while active builds a fresh processor and hands the old
one back to the plugin, in both exporters, so a processor that panicked recovers
at any activation; a reset asked of the processor it replaces is dropped, since
the new one has nothing to clear. A deactivation ends processing, though a host
may not have said so.

A setup whose block size is 0, or negative in VST3, keeps its sample rate and
takes 1,024 frames, and one past D72's limit takes that limit, a longer block
playing in pieces either way; one whose rate is not positive and finite is
refused. A VST3 activation never given a setup builds for 44.1 kHz and 1,024
frames.

**Alternatives.** The alternatives were an activation while active that does
nothing, and a setup with no block size refused.

[History](decision-history.md#d73-what-an-activation-makes-of-an-open-setup)

### D74. What export_vst3! refuses to build

**Question and C++ Seam.** Which plugins and platforms does `export_vst3!`
refuse to build? (D53, D58) JUCE's VST3 wrapper offers any bus layout the
processor accepts, and builds on Linux, where it runs its own message thread
(D53).

**Rust.** A main bus wider than stereo, naming the layout: the exporter arranges
a bus of one channel as mono and of two as stereo and nothing else, so it never
offers an arrangement it then refuses, and a wider plugin exports the C
interface for the JUCE adapter. Any platform but Windows and macOS, where it
would have no main-thread timer (D53). The exporter also refuses a plugin that
declares no layout, or a crate built with `panic = "abort"` (D60). The library
itself builds everywhere, so its tests and models run on Linux.

**Alternatives.** The alternatives were VST3's wider speaker arrangements built
properly, and on Linux a tick from the editor frame's run loop, from a thread of
the exporter's own, or none.

[History](decision-history.md#d74-what-export_vst3-refuses-to-build)

### D75. Making a caught fault visible

**Question and C++ Seam.** How does a fault the exporters catch become visible
outside a test? (D60) Not applicable: C++ caught nothing at the boundary (D60).

**Rust.** It is counted, and its kind reported once. Each instance counts its
faults from every thread, wait-free, the audio thread included, and
`seam_host::boundary_faults` counts the process's. The first fault of each kind
in an instance is written as one line naming the plugin and the instance, to an
attached debugger on Windows and to stderr elsewhere, at the times and on the
threads `crates/host/src/faults.rs` states, never on the audio thread; later
faults of that kind are counted, not written.

A fault outside any instance is written the first time its kind happens in the
process. The kinds: a panic answered as refused, a processor that panicked, a
call on the wrong thread or a release there, a call to an instance bound to no
thread that waited too long for another thread's call or a release that met one
(D62), a call refused while the plugin was inside one of its own calls to the
host, and a timer that did not start.

A panic's own message comes from std's panic hook, on the thread that panicked,
the audio thread included: the hook is the process's, so Seam leaves it to the
host.

**Alternatives.** The alternatives were a throttled line per instance and kind,
a line for every fault, or a callback the plugin installs.

[History](decision-history.md#d75-making-a-caught-fault-visible)

### D76. A panic while edits are told to the host

**Question and C++ Seam.** What does a panic leave queued while the exporter
tells the host of edits? (P9, P11) Not applicable: C++ had no panics, and its
gesture close swallowed exceptions (section 8).

**Rust.** Everything not yet told. If the call that hands an event to the host
panics during a drain, the parameter it was on keeps the flags whose events it
had not handed out and the queue is marked to be scanned again, so the next
drain hands them out and the host still hears a balanced bracket. `finish`,
which ends every bracket still open as the host stops listening, emits each end
before it marks the bracket closed, so an end that panics leaves the bracket for
the next `finish`.

**Alternatives.** The alternatives were to drop what a panic interrupts, or to
require the call not to panic, by documentation alone.

[History](decision-history.md#d76-a-panic-while-edits-are-told-to-the-host)

### D77. How deep a session section may nest

**Question and C++ Seam.** How deep may a session section nest? (S1, S4, H2) Not
checked against C++.

**Rust.** 125 levels (`seam_session::MAX_SECTION_DEPTH`; a scalar is level 0,
each array or object adds one), since `serde_json` reads 127 and the saved block
wraps 2 around a section. A deeper capture is refused as it is taken, so a save
never writes a block the same build cannot load: the participant's previous
section stays the saved one, the participant sees no error, and
`Registration::refused`, which is public, says so until a capture is accepted
again. A restored section whose re-capture is too deep stays as restored.

A capture that would grow the saved block past `seam_host::MAX_STATE_BYTES`
(64 MiB), the longest a load reads, is refused the same way, so a save never
writes a block longer than the same build loads. The saved state gives the
session a budget for its `extra` object: the limit, less the block's frame at
the longest version, each declared parameter at its longest value (24 bytes),
and the unknown keys the last load kept. Each section is measured once, as it is
captured or restored, by counting the bytes of its JSON text without building
the text. A capture that takes no more room than the section it replaces is
always taken. The session crate sits below the host, so the limit's value lives
in the session, hidden, and the host publishes it. The host sets the budget
through a hidden `Session::set_budget` at creation, and with the restored
sections at every load, through a hidden `Session::restore_within` that stores
both under one lock, so no capture is judged against the budget before the
load's.

A session's budget starts at the whole limit. Setting a lower one judges the
sections held already: the registered ones in registration order, then those no
participant holds in key order, and each that no longer has room is dropped, its
registration refused as at registration. A restored section still waiting for
its apply is dropped with its restore, so the participant keeps its state and
the apply saves nothing back. A participant registered before the saved state is
made, as the `Plugin` example attaches learn, is held to the same room as one
registered after.

A load refuses a block whole, with `TooLarge` and before it changes anything,
when the block it would write back would pass the limit: the declared
parameters at their longest, the unknown keys it keeps, and its `extra` as the
session measures it. A block from elsewhere can grow as it is written again
(`1e15` saves as `1000000000000000.0`, and a parameter it lacks is written in
full). A block this build saved loads again, since the load counts what the
capture budget counted.

Seam writes no log of its own.

**Alternatives.** The alternatives were to refuse the whole save, to raise
`serde_json`'s limit, or to tell the participant through an error from its
capture. For size, they were also to give each section a fixed share of the
block, to refuse a restore's sections one by one rather than the whole load, or
to require the saved state before any registration.

[History](decision-history.md#d77-how-deep-a-session-section-may-nest)

### D78. Protocol versions

**Question and C++ Seam.** How do a page and a native side of different protocol
versions find out? (B1, B4) No message carried a protocol version (B1).

**Rust.** The native side answers every `ready`: with a snapshot when it speaks
the page's version, and otherwise with a `reject` carrying its own version and a
reason, which fails the page. A `ready` decodes whatever fields a newer page
adds to it, so the reject always reaches that page; every other message still
refuses unknown fields (B4). A snapshot of another version, from a native side
that predates `reject`, fails the page too.

The protocol is version 2, and a test fails whenever the generated message
declarations change and the version does not: `crates/bridge/protocol-shape.txt`
holds one line per version, each with the hash of its declarations, and only
grows. One fixture of messages is read by the Rust tests and the page's, so both
ends agree on the text. No native side exists yet (D68); the decoding and the
page's half are built.

**Alternatives.** The alternatives were an empty snapshot behind a tolerant
`ready`, or `ready`'s shape frozen for good.

[History](decision-history.md#d78-protocol-versions)

### D79. The page client before the native side answers

**Question and C++ Seam.** What does the page client do while the native side
has not answered, and when a hook's command changes? (B1, B5) Not checked
against C++, which had no `ready` (B1).

**Rust.** The client sends `ready` again every second until a snapshot or a
reject arrives, it fails, or it is disposed, since the native side may not be
listening yet; the native side answers a repeat as it answers the first.
`useCommand` keys its pending count and its last result and error by the
command's name, so a change of name starts clean and a late reply to the old
name never shows as the new one's. The client is answered once it is ready or
has failed, so the repeats start only when neither came during the first
`ready`'s send.

The page holds every write and call until it is answered, so no write comes
before a repeated `ready`. The native side, when built (D68), may therefore
treat each `ready` as a new page session (B5) without telling a repeat from a
reload. That reading is the implementer's, confirmed by the owner on 2026-11-07.

**Alternatives.** The alternatives were one `ready` with a timeout that fails
the page, and a result kept across a change of name.

[History](decision-history.md#d79-the-page-client-before-the-native-side-answers)

### D80. Upgrading an older saved block

**Question and C++ Seam.** How does a plugin upgrade a block an older version of
it saved? (H1, H2) Not checked against C++.

**Rust.** A plugin declares its migration on its trait: `Plugin::SCHEMA_VERSION`
and `Plugin::migrate`, which Seam attaches to the saved state it builds (D91)
through `SavedState::with_migration(version, migrate)`. `version` is the
plugin's own schema version, `pluginStateVersion`, apart from Seam's frame
(D84), and a plugin with no migration is at 0. A save writes it, or the version
of the block it last loaded when that is higher, so a newer build's block that
an older build loads and saves back keeps its version, and the newer build does
not migrate it as an older one. The block keeps what the older build does not
read, too: the parameters it does not declare (D4), and the top-level keys other
than the versions, `parameters` and `extra`, saved after `extra` in key order.
The newer build then finds everything it wrote, under the version that promises
it. What it keeps counts against the load limit: a block whose kept keys would
write it back past `MAX_STATE_BYTES` is refused whole (D77).

A block of an older plugin version is handed to `migrate` whole, with that
version, as a JSON object, to rewrite in place before it loads; a migration adds
what the block lacks and never overwrites a key the block holds, which may be
newer than anything it derives. A migration that cannot upgrade the block
returns its reason, and the load fails with it and changes nothing (D85); a
block of the plugin's version or a newer one loads as it is (H2). It runs inside
the load, on the loading thread, so it touches only the block, and
`Plugin::migrate` has no `self`.

One hook keyed by the block's version, not one per section; learn's section
carries its own schema (L9).

**Alternatives.** The alternatives were a version and a hook per session
section, or none until a plugin needs one.

[History](decision-history.md#d80-upgrading-an-older-saved-block)

### D81. A registration made while a panic unwinds

**Question and C++ Seam.** Does a late apply made while a panic unwinds leave
the session owing a pass? (S6, S10, D55) Not applicable: C++ had no panics.

**Rust.** No. A pass or a late apply that a panic unwinds did not finish, so the
session owes the pass again and the next tick runs it, which keeps a later
restore from being stuck. Only a panic that began inside it counts. A
registration made from a destructor while another panic unwinds runs its late
apply there and owes nothing, since a pass owed for nothing would mark a
restore completed that never came and clear every undo history (S10). A panic
inside that late apply aborts, as any panic does while another unwinds.

**Alternatives.** The alternative was to owe a pass whenever one ended during a
panic, which clears every history after such a registration.

[History](decision-history.md#d81-a-registration-made-while-a-panic-unwinds)

### D82. When learn runs a target's own code

**Question and C++ Seam.** When do a restore and an unmap run a learn target's
own code? (L8, L9, D54, D71) Not checked against C++.

**Rust.** Last. A restore ends the arm, clears every mapping and held entry,
empties each target's staging without calling it, then decodes and inserts the
section's mappings, publishes the table and tells the session. Only then does
it close each open burst, which runs the target's `end`. An unmap removes the
mapping, empties its staging, publishes and tells the session before it closes
the burst, and closing a burst does not depend on the staging. So a target
whose `end` panics leaves no earlier mapping routing, in learn or on the audio
side, and learn stays usable.

**Alternatives.** The alternatives were to close the bursts first, which a
panicking `end` stops with the previous project's mappings in place, or to
clear first and close before publishing, which leaves the audio side routing a
table learn no longer lists.

[History](decision-history.md#d82-when-learn-runs-a-targets-own-code)

### D83. A control's drag and one-shot set

**Question and C++ Seam.** Do a page control's `onDrag` and `setValue` differ?
(B2) Not checked against C++.

**Rust.** Only in the gesture around them. `useParam` returns one function as
both: it limits the position to 0 to 1, shows it and sends a `set`.
`onDragStart` and `onDragEnd` put a drag's gesture around the sets made between
them, so a `setValue` during a drag joins the drag, and one outside a drag is a
one-shot set, whose gesture the native side is to open (B2).

**Alternatives.** The alternative was two functions, which their names would let
drift apart.

[History](decision-history.md#d83-a-controls-drag-and-one-shot-set)

### D84. The saved block's two versions

**Question and C++ Seam.** Whose version does the saved block carry? (H1, H2,
D80) The C++ Seam wrote one, `seamStateVersion` (H1).

**Rust.** Two, each its own. `seamStateVersion` is the version of Seam's frame,
the shape Seam gives `parameters`, `extra` and the keys a load keeps:
`STATE_VERSION`, at 1. `pluginStateVersion` is the plugin's own schema, the
version `with_migration` names (D80). A plugin with no migration is at 0, and a
block without the key reads as 0. A save always writes both, the plugin's at 0
too, so every block says what it holds, in this order: `seamStateVersion`,
`pluginStateVersion`, `parameters`, `extra`, then the kept keys in key order
(H1).

A block from a newer build loads what this build reads. Each version saves back
as the higher of the block's and this build's, on its own, so neither lifts the
other, and the keys a load keeps are kept as before (D80). A frame version that
is missing, not a whole number or below 1 makes the block unrecognised, and so
does a plugin version that is there but not a whole number from 0 (H2). Seam
would migrate its own frame first, and the plugin's migration after it; the
frame has had one shape, so only the plugin's runs, when the block's plugin
version is below the plugin's.

**Alternatives.** The alternatives were one number for both, which Seam could
never raise for its own frame, and a plugin version left out at 0, which a
reader would have to know means 0.

[History](decision-history.md#d84-the-saved-blocks-two-versions)

### D85. A migration that cannot upgrade a block

**Question and C++ Seam.** How does a migration say it cannot upgrade a block?
(D80) Not checked against C++.

**Rust.** `migrate` returns `Result<(), Box<dyn Error + Send + Sync>>`, so it
can use `?` on any error, and give text with `.into()`. The load then fails with
`LoadError::MigrationFailed`, which holds the reason and returns it as its
`source`, and changes nothing. So `LoadError` is neither `Copy` nor `PartialEq`,
and a test matches on its variant. Each exporter answers the host as it did for
any failed load, and writes the reason on a line, once per instance (D66).

**Alternatives.** The alternatives were a closure generic over its error type,
which neither `Ok(())` nor `?` lets the compiler infer, so every migration would
name its error type, and the `bool` it returned before, which dropped the
reason.

[History](decision-history.md#d85-a-migration-that-cannot-upgrade-a-block)

### D86. The AU codes

**Question and C++ Seam.** How does `PluginInfo` hold the AU codes? (D64) Not
applicable: the C++ Seam's plugins were JUCE's, whose build names them
`PLUGIN_CODE` and `PLUGIN_MANUFACTURER_CODE`.

**Rust.** `PluginInfo::au_codes` is one `AuCodes`, whose `subtype` and
`manufacturer` fields are four bytes each, so a plugin that swapped them says so
where it writes them. The JUCE adapter's settings take the subtype as JUCE's
plugin code and the manufacturer as its manufacturer code (D64).

**Alternatives.** The alternatives were two fields on `PluginInfo`, which split
one identity across the struct, and the pair `([u8; 4], [u8; 4])` it was, whose
order nothing checked.

[History](decision-history.md#d86-the-au-codes)

### D87. The main bus in `ProcessSetup`

**Question and C++ Seam.** How does `ProcessSetup` give the main bus? (D58)
JUCE's `prepareToPlay` took the rate and the block size; the channel counts came
from the processor's buses.

**Rust.** As one `layout: Layout`, the one of `PluginInfo::layouts` the host
chose, which every exporter checks before it activates. The C interface's
`SeamLayout` and `seam.h` are unchanged: only the Rust type is.

**Alternatives.** The alternative was the two counts, `inputs` and `outputs`, it
had, which a processor put back together by hand.

[History](decision-history.md#d87-the-main-bus-in-processsetup)

### D88. A call a test host was refused

**Question and C++ Seam.** What does a test host answer for a call the plugin
refused? (D60) Not applicable.

**Rust.** `Result<(), Refused>`, for every call the plugin can refuse.
`seam_host::vst3::Refused` carries the `tresult` the plugin answered and names
it when shown; `seam_host::cabi::Refused` stands for the C interface's `false`,
which carries nothing more. A test then pins the exact answer, or uses `?`.
`save` answers `Result<Vec<u8>, Refused>` too, so the VST3 host's carries the
`tresult` `getState` answered. A lookup (`parameter`, `text`, `parse`) answers
`Option`.

**Alternatives.** The alternatives were the `bool` each returned, which mirrored
the `tresult` as a C++ host would, and one `Refused` for both hosts, whose
variants each host would only half use.

[History](decision-history.md#d88-a-call-a-test-host-was-refused)

### D89. How plugin code names a parameter

**Question and C++ Seam.** How does plugin code name a parameter and read its
value? (P1, P10) Not applicable: the C++ Seam's plugins were JUCE's, and read
their parameters through JUCE.

**Rust.** Plugin code holds a parameter as a typed parameter: `Float` for a
continuous one, `Bool`, or `Choice<E>` for an enum `E` that implements
`Choices`, whose `LABELS` the declaration takes as `Kind::Choice(E::LABELS)`.
Each carries its store: the parameter's one cell, which its set shares, with
the value word and the change count first and the id and declaration behind
them. `get` is one atomic load and a decode. It names no set and has no path
that panics; for a choice it calls `Choices::from_index` on a saturated index,
and that must be total too.

A parameter is `Clone`, and a clone is the same parameter: equality is the
store's identity, not the value. It is not `Copy`, since the last drop frees
the store, so its drop is checked against the audio-thread mark (D67).

`ParamSet::bind` makes one from a key, once, when the plugin is built. It
refuses with a `BindError` that names the key when nothing is declared under
it, when the declaration is another kind, and when a choice's labels are not
`E::LABELS`. Nothing is checked after that. A parameter's `id()` is its place
in the set, by which every call that takes a `ParamId` names it, and `value`
on the set and its handles takes a `ParamId` and returns the stored `f32`.
`UiParams::write` takes a typed parameter, by reference, with a value of its
type, or a `ParamId` with an `f32`, through one sealed trait. It refuses a
parameter of another set: `false`, and a debug assertion. Declaring a
parameter with a typed declaration hands it back (D90); `bind` serves a plain
table of `ParamDecl`s.

A processor asks once per block whether any of a group of parameters moved
with `Changes`, which holds a stamp and no parameter. The members come with
each call, as `moved([&gain, &mode])`, and may be of different kinds. The
stamp is the sum of their change counts and a mix of their stores' serials,
numbers no two stores share in a process. So a call about other parameters
than the last answers true, short of a collision in the mix, which is chance,
about once in 2^64. The check is a load per member and a compare.

**Alternatives.** The alternatives were the id it was, looked up by string
behind `expect` and decoded with a threshold per kind in every plugin; a handle
that is an index into its set, which is read against that set, reads another
parameter or panics when paired with another set, and cannot exist before its
set does; typed reads under names of their own beside the untyped ones, which
doubled the API; a `Changes` that holds its members, which ties it to one
group's size; and a group check over a boxed list of ids, which puts a pointer
to follow on the audio thread.

[History](decision-history.md#d89-how-plugin-code-names-a-parameter)

### D90. How a plugin declares its parameters

**Question and C++ Seam.** How does a plugin declare its parameters and come
to hold them as typed parameters? (P1, P10, D89) Not applicable: the C++
Seam's plugins declared JUCE's parameters.

**Rust.** In one function that returns them. A struct of parameters
implements `Params`, whose `declare(p: &mut Declarer) -> Self` adds each
parameter and keeps what it is handed: `p.add(FloatDecl::new("gain", "Gain",
-60.0..=12.0, 0.0).with_step(0.1))` is a `Float`. A declaration is typed, and
its type is the kind: `FloatDecl` makes a `Float`, `BoolDecl` a `Bool`, and
`ChoiceDecl<E>` a `Choice<E>` whose labels are `E::LABELS` and whose default
is an `E`. A field of another kind does not compile, and only a `BoolDecl`
can be the bypass. A struct may hold another and call its `declare`; `()` is
the struct of a plugin with none. `ParamSet::declare::<P>()` runs `declare`
and freezes the set.

`add` makes the parameter's store there and then. Its id, its place in the
order hosts list parameters, is fixed when the set is frozen, as a table's
is (P10), since the set orders parameters by group. Asking for it inside
`declare` is a contract panic.

A mistake is a compile error on the author's own line where it can be. Each
typed declaration has a `const fn checked` that panics on the rules one
declaration can break alone: the key's, the group's, the labels', the
range's, and a float's default inside its range. In a named `const` that
fails `cargo check`; an inline `const { }` block fails only when code is
generated. Those rules are written once, as `const fn`s, and
`ParamSetBuilder::add` and `Declarer::add` call the same ones. What only the
whole set shows (a key, group name or host id another parameter has, a
second bypass) is one `DeclError` when the set is built. `declare` returns
no `Result`: the declarer keeps the first refusal and goes on, handing back
parameters with stores of their own and in no set, and `ParamSet::declare`
returns the refusal and drops what `declare` returned.

A typed declaration refuses a default outside its range, and a choice's
default past its labels, with `DeclError::DefaultOutsideRange`; a
`ParamDecl` still snaps its default.

The table path stays: `ParamDecl`, `Kind`, `ParamSetBuilder` and
`ParamSet::from_decls` keep their signatures and the rules they apply, and
`Declarer::extend` declares a table, for a test plugin or one whose
parameters come from data. A plugin names its struct as `Plugin::Params`,
and Seam declares it when it builds an instance (D91). Since
the builder shares the typed declarations' rules, it applies the ones a row
breaks on its own before the ones only the rows before it show, so a row
that breaks two rules is refused for the one of its own: a repeated key
whose choice has one label is `TooFewChoices`, not `DuplicateKey`.

**Alternatives.** The alternatives were a table of `ParamDecl`s beside typed
fields bound by key, which writes each parameter four times (a key, a row, a
field and a bind) and says its kind twice; typed constants gathered in a list
and bound, which cannot be gathered without naming each one again, since a
`const` must state its type and a `const fn` cannot call a trait method; a
`Result` from `declare`, which every plugin would pass on; an id fixed when a
parameter is added, which a parameter that carries its store does not need;
and a typed declaration that snaps its default as a table row does, which
hides a typo.

[History](decision-history.md#d90-how-a-plugin-declares-its-parameters)

### D91. Who builds a plugin's parts

**Question and C++ Seam.** Who builds a plugin's parameter set, session and
saved state, and how does the plugin get them? (D51, D52, D53, D90) Not
applicable: the C++ Seam's plugins built them in their own constructors, as a
JUCE `AudioProcessor` does.

**Rust.** Seam does, once per instance, and moves them into the plugin.
`Plugin::Params` names the struct that declares the parameters a host is shown
(D90). An exporter calls `Parts::for_plugin::<P>()`, which declares the set
with `ParamSet::declare`, makes the session and the saved state over it,
attaching the plugin's migration (D80), and hands `Plugin::new` the parameters
and the `Parts`. The exporter keeps the set and the saved state, which are
`Send + Sync` (D51). A declaration refused fails the build: the exporter writes
a line with the plugin's name and the reason, then panics inside its guard, so
the host is refused the instance as for a panic in `new` (D60, D75).
`juce_settings` reads the keys from a set declared the same way.

The plugin owns its `Parts`. They hold the main thread's handles: the set's
and the session's UI handles, and undo's history and learn once the plugin
turns them on with `undo` and `midi_learn`. So a plugin that keeps them is not
`Send`, and one that must be drops them and keeps its parameters, as the
example EQ does. Learn sits in a shared cell inside the parts, since the
session holds it as a participant; `Parts::learn` lends it, and plugin code
never names `Rc` or `RefCell`. `midi_learn` panics for a plugin that declares
no MIDI input, since no controller would reach learn, and for learn turned on
twice; `undo` panics while a history the plugin made itself is alive. Neither
returns a `Result` (D95).

Seam ticks the parts, just before each `Plugin::idle`, so a plugin never
does, and `idle` holds only the plugin's own work. The engine makes a `Driver`
from the parts before `Plugin::new` takes them: a weak link to what the tick
needs, so a plugin that dropped its parts is not ticked. `Driver::tick` runs in
the one order that works: learn's poll, so a mapping it commits reaches the
same tick's save; the session's tick, which applies a restore (D13); the
history's (D20); then the capture of the saved plain values (D92). It holds
nothing of the parts while the session's tick runs, so a participant may reach
back into them from its apply; learn is held while it polls, as before. When
nothing changed it allocates nothing
and does no work that grows with the number of parameters. `idle` then sees a
restore applied and a learned mapping committed. Both run inside the plugin's
borrow, so a tick that meets the plugin busy in one of its calls runs neither.
The driver is not `Send`, so the engine keeps one, beside the timer, only for
a plugin bound to its thread; a plugin that is `Send` holds no parts. A test
with no host makes a `Driver` from the parts and ticks through it.

**Alternatives.** The alternatives were each plugin building its parts and
handing the saved state back through a getter, `Plugin::saved`, which wrote
the same construction in every plugin and tied nothing to the table the trait
named; and the engine owning the parts and ticking them, lending them to each
call. Through the C interface a `Send` plugin's calls may arrive on different
threads, and a clone of a handle the plugin kept on the first would then be
used on another, so that is unsound. Seam's tick leaves ownership alone: the
plugin keeps its parts, and the engine reaches them only on the plugin's own
thread.

The plugin calling `Parts::tick` from `idle`, with the engine reporting, once,
a plugin that kept its parts and let an idle pass without the call, was built
first and left. A forgotten call lost a project's session on load with no sign
but a line in a host's log, a plugin's own tests never saw it, and every
plugin repeated a call into the framework's share of the work. The tick
returning what learn committed was left with it: nothing read it, and a plugin
asks learn for the mapping. A signal that learn committed a mapping comes back
with the page, whose context menu learns and clears a mapping; it decides
between a hook on `Plugin` and a change count on learn that the page polls.

[History](decision-history.md#d91-who-builds-a-plugins-parts)

### D92. When a saved plain value is captured

**Question and C++ Seam.** When does a plain value saved through
`Parts::saved_value` reach the session, and how does Seam know it moved? (D44,
D91)
The C++ Seam had no such call: each owner of saved state was a participant
that told the registry of each change itself, as a participant a plugin writes
by hand still does (D44).

**Rust.** At the tick. `Parts::saved_value` makes the value, starting at the
default it is given, and registers a participant Seam owns for it: it captures
the value as JSON through serde, and applies a restored section, or the same
default for a project without the key, so a new instance and an older project
start alike.
`Driver::tick` ends with `Parts::sync`, which compares each saved value's change
count with the count it last saw, one atomic load, and captures the value when
it moved. When the newest write was the value's own apply, the session's apply
pass captured it as it applied, so `sync` notes the count and captures nothing:
a load is captured once. A write another participant's apply makes after it is
a restore's too, and is captured as any other write. So a save between a
change and the next tick, about 16 ms, holds the value from before the change,
and `sync` captures at once for a caller that cannot wait; a parameter is
never stale this way. The
whole value is captured whenever it moved, so large state saves under several
keys. A section serde cannot read as the value's type fails its apply, as any
participant's does (D14). A value serde cannot write as JSON fails a debug
assertion where it is captured; a release build saves nothing under its key.

**Alternatives.** The alternatives were capturing at each write, which needs
the value to know what saves it, so a type of its own for the author to hold;
and a participant written by hand, as the example gain had, which calls
`Registration::changed` after every write, undo and redo included, and holds
the old value for good where one call is forgotten. The call was first
`Parts::save(key, &value, default)`, over a value the plugin made: the default
was written twice, as the value's start and as what a project without the key
loads, and nothing made the two agree. Keeping it beside `saved_value` would
have left that trap for whoever used it, and nothing needs it: a value is a
shared handle, cloned to another owner; a default that differs on purpose, so
an older project keeps its behaviour, is what `Plugin::migrate` is for; and
anything else is a participant written by hand. `save` without its default was
the other form left: two calls still, and a value written before the call
would load that write for a project without the key.

[History](decision-history.md#d92-when-a-saved-plain-value-is-captured)

### D93. A derive that writes `declare`

**Question and C++ Seam.** How does a plugin declare a struct of parameters
without writing `declare`, and where is a mistake in it reported? (D90) Not
applicable: the C++ Seam's plugins declared JUCE's parameters.

**Rust.** `#[derive(Params)]` writes the `Params` impl a person would write:
one `declare` that adds each marked field's declaration, in field order, and
returns the struct. Nothing else is generated, so a derived struct may hold one
written by hand and the other way round. Inside a field's `#[param(...)]` mark,
the leading items are the arguments of the declaration's `new`, as written;
`name = value` is its `with_name(value)`; a lone lowercase word is its
`as_word()`. `true` and `false` are values, and a constant is written in
capitals. An argument after an option is the derive's own error, and so is an
option or a flag no declaration has, which it names with the ones there are; an
option or a flag the field's kind lacks is the compiler's "no method" error.

The kind is the field's type: the sealed `Declared` trait gives `Float` its
`FloatDecl`, `Bool` its `BoolDecl` and `Choice<E>` its `ChoiceDecl<E>`, and the
derive names the declaration through it, never through the type's name, so a
type alias works. A field with no mark is a struct of parameters and declares
itself through its own `Params`.

Each declaration is a named constant, in a block of its own, that ends in
`checked`, so a key, a range or a label no parameter could have fails a plain
`cargo check` (D90), and one more constant per field refuses a key an earlier
field of the struct has. Each generated token carries the span of what it came
from, so every error lands on the author's mark or field. A key two structs
share is refused when the set is built, as by hand.

An enum, a union, a tuple or unit struct and a generic struct are refused.
Arrays, optional parameters and pools are written by hand: the example gain
derives its parameters, and the example EQ writes `declare` itself, since its
bands come from a table. The derive lives in `seam-macros`, a proc-macro crate,
and `seam-params` re-exports it under its `derive` feature, off by default, so
the core still builds with no dependency.

**Alternatives.** The alternatives were a `params!` table macro, whose body
rustfmt leaves alone and which reports an unknown kind badly; an inline `const
{ }` block per declaration, which `cargo check` does not evaluate; matching on
the field's type name, which a type alias defeats; splitting the mark at its
commas, which breaks an argument with a comma inside generic brackets; and
leaving a misspelt option to the compiler, whose hint names the method,
`with_step`, where the mark wants `step`.

[History](decision-history.md#d93-a-derive-that-writes-declare)

### D94. A derive that writes `Choices`

**Question and C++ Seam.** How does a plugin name a choice's values without
writing `Choices` by hand, and what does the derive refuse? (D89, D93) Not
applicable: the C++ Seam's choices were JUCE's string arrays.

**Rust.** `#[derive(Choices)]` writes the `Choices` impl a person would
write, from the enum alone: `LABELS`, and `from_index` and `index` as matches,
all three in the order of the variants, so the three cannot drift apart as
they can by hand. A variant's label is its name, unless a `#[choice("...")]`
mark on it gives one. A label is shown and never saved, so renaming a variant
changes only what a host shows. `from_index` gives the last variant for an
index past the end.

A discriminant the author writes must equal the variant's position: a choice
is saved as its index, every hand-written impl used `self as usize`, and the
derive must never disagree with the cast silently. One constant per written
discriminant, spanned on it, compares the two, so moving or inserting a
numbered variant fails `cargo check` there. An enum whose discriminants mean
something else writes `Choices` by hand. Each label is checked once, by a
constant spanned on its mark, or on the variant's name when that is the
label, that applies the rules a choice's declaration applies to one label
among the earlier ones (D90). So a bad label is refused on itself, where the
enum is declared, and not on each declaration over it. A struct, a union, a
generic enum, an enum of fewer than two variants, a variant with fields, a
second mark on a variant, a mark that is not one string and a mark on the enum
itself are the derive's own errors.

The derive cannot keep a saved project right when the order changes: the
order of the variants is the save format. Its doc and the guide say so. A
variant added at the end keeps saved projects right but moves every host
position; a variant moved or inserted needs a higher `SCHEMA_VERSION` and a
`migrate`. It lives in `seam-macros` beside `Params`' derive and is
re-exported under the same feature. The `Choices` trait's own example stays
hand-written, where the contract is shown, and builds without the feature.

**Alternatives.** The alternatives were a mark required on every variant,
which writes each label twice for the common case where the name is the
label; a written discriminant ignored, which lets the derive's index and the
enum's cast disagree unseen; and the labels checked only at each declaration
of a choice over the enum, which reports one mistake on every `#[param]` mark
that uses it, and never for an enum no declaration uses yet.

[History](decision-history.md#d94-a-derive-that-writes-choices)

### D95. Mistakes panic, failures return a `Result`

**Question and C++ Seam.** When does a call at the plugin boundary panic, and
when does it return a `Result`? (D90, D91) Not applicable: the C++ Seam's
plugins set up JUCE's parts in their own constructors.

**Rust.** A mistake fixed in the caller's source, such as a call made twice or
a switch turned on without what it needs, panics, with what to change in the
message. It shows the first time the code runs and never depends on a user's
data. A failure that can depend on data at run time, such as a key built from
a name or a block a host hands back, returns a `Result`. It is D90's rule for
declarations applied to setup: a mistake there is one error when the instance
is built, and plugin code has nothing to handle. A panic in `Plugin::new`
refuses the instance and writes its line, as a refused declaration does (D60,
D75).

Applied where the plugin's parts are set up: `Parts::undo` returns the
`History`, and panics while a history the plugin made over `Parts::params` is
alive; a second call still returns the same history. `Parts::midi_learn`
returns nothing, and panics when learn's section key is taken, by learn turned
on twice or a participant the plugin registered, and for a plugin that
declares no MIDI input. `Parts::saved_value` keeps its `Result`, since its key
is a string a plugin may build at run time. The core crates keep their
signatures, `History::enroll` and `Learn::arm` among them: the rule is written
in `docs/style.md` for what comes next, and applied where the plugin boundary
already changed, not swept through the workspace.

**Alternatives.** The alternatives were a `Result` from every call, so each
call in `new` took an `expect` with a message the author had to invent for a
mistake in their own source, and the calls disagreed: `midi_learn` panicked
for a plugin with no MIDI input and returned an error for a taken key, `learn`
panicked while learn was off, and `undo` returned an error; and `Plugin::new`
returning a `Result`, so that each `expect` became a `?`. Every plugin would
then pay `Result` and `Ok`, most for calls that cannot fail, every `impl
Plugin` would break again, and refusing to build an instance on a run-time
failure, a missing file say, would become the easy path, with a host then
opening the user's project without the plugin.

[History](decision-history.md#d95-mistakes-panic-failures-return-a-result)

## How to read this

Each rule has an id, the **invariant**, **why** it exists, and its **Rust**
form. Where a test in the C++ suite pins a rule, it is named after "Pinned by",
so the behaviour can be checked against the old code. File references are to
`seam-cpp` at `149005a`; those to a JUCE file (`juce_*`, `NormalisableRange.h`)
are to JUCE 9.0.0 (`f8f8864`), the version `seam-cpp` builds against by default.

These are the rules. A rule whose Rust form changed links its history. The
decisions D1 onward are in the Overview and in the section 'The decisions in
full', and use another shape.

Terms:

- **Store**: a parameter's one value cell, in real (not 0..1) units.
- **Origin**: who wrote a value: the host, a restore, a learned controller, a
  user, the plugin's own code, or an undo replay.
- **Surface**: one presentation of a value (a control on the page, a native
  widget).
- **Gesture**: a user's touch on a value, from pointer down to pointer up.

---

## 1. Parameters (seam-params)

### P1. One store per parameter

**Invariant.** Each parameter has exactly one value cell, in real units, written
and read atomically without locks. The audio thread, the UI, the host's read and
the save path all read that cell, so none of them sees a value older than
another. Concurrent writers are last-write-wins; reads are consistent per
parameter, never across parameters.

**Why.** A separate audio-thread mirror once lagged a UI edit by a message tick
(commit `e3980d1`); making the atomic the only store removed the whole class of
desync (`3533b6a`).

**Rust.** One `AtomicU64` per parameter, `Relaxed`, holding the value's f32 bits
with the origin of the write that stored them (D38); ordering lives on the
change counter (P7). One store writes both and one load reads both, so a UI
write racing a host write never leaves one's value beside the other's origin; a
target without 64-bit atomics does not compile. The host's 0..1 value is
computed from the cell, never cached.

### P2. Every write snaps, per kind

**Invariant.** All public values are real units; 0..1 exists only at the host
boundary. Every write from every origin passes the kind's constraint:

- **Continuous**: the legal values are the lattice points `start + k * interval`
  up to `end`, plus `end` itself. A write goes to the nearest legal value, a tie
  to the higher one. Skew affects only the 0..1 mapping, never snapping. Example
  `{0, 10, step 3}`: 7.4 → 6, 8 → 9, 10 → 10, 11 → 10.
- **Bool**: `>= 0.5` is 1, else 0.
- **Choice**: round to the nearest index, a tie to the even one (as C++ did),
  clamp to `0..=n-1`. The 0..1 span is floored at 1 so a degenerate choice
  cannot divide by zero. At least two choices.

**Why.** An off-lattice value must never reach the DSP from anywhere, the host
included. C++ snapped to the lattice and then clamped, which is not the rule
above (`seam_RangedReactive.h:45` into JUCE's `NormalisableRange.h:193-196`).
When the end sits less than half an interval past the last lattice point,
writing the end, a host 1.0 or a reload lands on that lattice point: in `{0, 10,
3}`, 10 snapped to 9, a host write of 1.0 landed on 9, and a saved 10 reloaded
as 9. An end half an interval or more past it stayed reachable. Found in this
transfer; no C++ test wrote the end itself. A choice rounded with
`juce::roundToInt`, which keeps the low 32 bits: a write between 2^31 and 2^32
wrapped negative and clamped to the first choice, and a larger one could land on
any index (`seam_ChoiceParameter.h:99`, `juce_MathsFunctions.h:597-609`).

**Rust.** Pure `fn(f32) -> Option<f32>` per kind, `None` meaning refused
(non-finite), so each origin applies its own fallback (P3). Snapping is
idempotent (property 1 in section 7 checks it). The lattice is laid out from the
decimals the range was written with, in whole numbers of its finest place (D57).
Fewer than two choices is a builder error in every build (C++ only asserted in
debug).

Pinned by: `parameter_test` "writes snap to the stepped lattice and the clamped
endpoint stays reachable"; `host_parameter_test` "an off-lattice host setValue
snaps before reaching the DSP atomic"; `choice_parameter_test` "the choice-count
span floor keeps a degenerate range NaN-safe".

### P3. A non-finite value never reaches the store

**Invariant.** Each origin has a fixed fallback for NaN or infinity:

| Origin | Fallback |
| --- | --- |
| Host (0..1) | 0, which is the range start; +inf lands at the start too |
| User or program | refused: no store, no notification |
| Restore from a saved block | the parameter's default |
| Learned write | ±inf lands on the range's ends, since its 0..1 value is clamped before conversion; NaN keeps the current value |
| Declaration | the range start, for the value and the default |

**Why.** A stored NaN defeats the equality short-circuit (NaN != NaN), so it
re-announces forever, and it poisons the DSP. The fallbacks differ because each
is what that origin's caller can observe and expects. C++'s learned write
converted with JUCE's `convertFrom0to1`, which clamps first
(`seam_MidiParameterTargets.h:120`, `NormalisableRange.h:162`), so only NaN
reached the refusal. Its user and program refusal held absent a concurrent
writer: it substituted the current value and `set` re-read it, so a write
landing between the two reads made the refused write store the older value
(`seam_RangedReactive.h:43`, `seam_ReactiveBase.h:164`). Rust's refusal is a
`None` and stores nothing.

Pinned by: `anythread_set_test` "set(NaN/inf) is a no-op on every constrained
kind"; `host_parameter_test` "setValue with non-finite input leaves a finite
realtime mirror".

### P4. Defaults are legal values

**Invariant.** A default is snapped like any write (non-finite falls back to the
range start), and the store starts there. The host's default is its 0..1
projection.

**Why.** An unsnapped default reached the host and the double-click reset.

### P5. "Changed" means the snapped value differs from the store

**Invariant.** A write is a change if its snapped value is not equal to the
stored one. A no-op write does nothing: no store, no counter bump, no host
event. The check a control makes before opening a gesture is the same function,
so a click that changes nothing opens no gesture and leaves no undo step.

**Why.** Empty host gestures and empty undo steps on clicks that changed nothing
(commit `230ed9a`).

**Rust.** Expose the predicate the write itself uses. Compare a gesture's user
writes against the value the gesture shows, the last UI-side write in it, edits
and replays included, not the shared store: in C++, during a drag, a user write
equal to a host write not yet shown was a no-op, and undo's after-value missed
it.

### P6. Write origins decide every side effect

**Invariant.**

| | Host-shaped: host automation, restore, learned controller | User edit | Plugin's own code |
| --- | --- | --- | --- |
| Thread | any, the audio thread included | UI | UI for an edit, any but audio for a sync (D28) |
| Snaps | yes | yes | yes |
| Host told | never | begin, value, end | see D1 |
| Gesture | never | yes, counted (P11) | an edit joins the count (D28); a sync never |
| Undo | never | one step per gesture | never for a parameter |
| Surfaces see it | on their next poll, held back while a gesture is open (P12) | at once | an edit at once; a sync on the next poll, held back while a gesture is open (P12, D34) |
| Must never | block, allocate, make a system call, echo to the host, open a gesture | run on the audio thread, open a gesture for a click that changes nothing (P5, D32) | run on the audio thread, record a parameter step |

An undo or redo replay is a user edit to the host (one complete gesture) and is
never recorded (U9). A restore and a learned write use the real-units write
(P8); host automation uses the 0..1 write (P7).

**Why.** Host automation must never become undo history nor bounce back to the
host. A write from a worker thread must not reach a host call that is
main-thread-only, nor pass for a user edit (commit `81978b0`). A restore must be
silent and leave undo clean. A learned controller must reach the DSP in the
block it arrived in without touching undo.

**Rust.** The origin is part of the type: the audio thread holds a handle that
can only make host-shaped writes; the UI handle's user write needs a gesture
token or a one-shot call; a program write states its intent (D1). C++'s "only on
the UI thread" gate on host notification was a VST3 artefact. In Rust an edit
runs on the UI thread, and a sync's re-read request is a flag each exporter
reads on its own main thread (D28), so the gate goes and the intent replaces it.
The types that lock (a plain value, a list, the session, a load) are `Send` and
`Sync`, so the compiler cannot keep them off the audio thread; a debug build
checks it with the audio-thread mark the exporters hold around the block (D67).

Pinned by: `undo_host_param_test` "host automation on an enrolled parameter
records NOTHING"; `host_parameter_test` "a UI edit notifies the host; a host
change does not bounce back", "an off-thread app set() stores and notifies
synchronously but never notifies the host"; `state_persistence_test` "a full
restore changing every value fires zero host notifications, before and after a
pump"; `midi_membrane_test` "midi: a mapped controller reaches the DSP in the
block it arrived in".

### P7. The host write: sanitise, clamp, convert, snap, store, publish

**Invariant.** A host write replaces a non-finite 0..1 value with 0, clamps to
0..1, converts to real units, snaps (P2), stores, then publishes the change. It
is constant-time and never blocks, allocates, makes a system call, tells the
host or tells a surface. It does no equality check: every host write stores and
publishes; surfaces deduplicate.

Orderings: the value store is relaxed and sequenced before the publish, which is
a release. A reader acquires the publish, then loads the value relaxed, so it
reads the value that preceded the publish or a newer one. No update is lost: a
write published after a reader's acquire is seen by its next poll.

**Why.** The write may run on the audio thread. An earlier version posted a
message per write, a lock and a system call per automation point (removed in
`88fae7d`).

**Rust.** Publish is a per-parameter `AtomicU64` change counter,
`fetch_add(Release)` after the value store. C++ used a boolean that the UI poll
consumed with an exchange. A consume-on-read flag starves every reader but the
first, which is why C++ needed one shared poller per value; that poller's one
drain then announced to every subscriber (`seam_HostParameter.h:313-323`). A
counter lets every surface keep its own position. A `loom` model checks the
count-then-value read, and fails when either ordering is weakened to relaxed.

Pinned by: `host_parameter_test` "the DSP atomic survives concurrent host writes
and lands the final value"; `plugin_processor_test` "an audio-rate host burst
does not fan out onChanged; the drain reconciles it once".

[History](decision-history.md#p7-the-host-write-sanitise-clamp-convert-snap-store-publish)

### P8. The restore and learned write: real units, silent

**Invariant.** A restore writes real units directly, with no 0..1 round trip:
snap, store, publish. A learned controller converts its 0..1 value to real units
once and writes the same way. Both have every guarantee of P7.

**Why.** Real to 0..1 to real is not the identity in float on a skewed range,
which would break "a saved block round-trips exactly".

Pinned by: `state_persistence_test` "save -> restore -> save is byte-identical
on a linear, a skewed, and a stepped range".

### P9. One loud write path

**Invariant.** User and program writes go through one place, in this order:
snap; if unchanged, stop; store; queue the host event (user and program edits; a
sync raises the re-read flag instead, D28); publish to surfaces. Store comes
before anything can observe the change. Host-shaped writes never pass through
it, which is structurally why automation cannot echo back to the host. The host
event carries the snapped value.

**Why.** Store-before-notify: no reader ever sees a notification older than the
value (`e3980d1`). One path means one place to get the order right. For user and
program writes C++ had exactly one announcing method, `set`, and the type system
kept every other path silent; the projection of a host write announced through a
second one, `notifyChanged` (`seam_ReactiveBase.h:297`).

**Rust.** The C++ rules about announcing last, from a stack copy, so a listener
could destroy the value mid-announcement, are moot: nothing is announced
synchronously. A host callback failure never unwinds into the core: exporters
catch panics at the boundary. Every write, a sync's and a host-shaped one's
included, stores and publishes through one `ParamSet` method that takes the step
between the two (telling the host, or asking it to re-read) as a closure, so the
order is written once. A `loom` model races a user write and a sync against a
reader of the count and of the re-read request, and fails when that method
publishes or takes its step before it stores.

### P10. Identity and order

**Invariant.**

- A parameter's identity is a stable key: never the display name, not empty, no
  `/`, unique across the plugin, at most 31 characters (AAX truncates longer
  ones and would merge two that share a prefix).
- Group names and parameter keys share one namespace. Groups are presentation
  only; lookup is by key, so moving a parameter between groups keeps sessions.
- The order is depth-first, a group's own parameters before its subgroups, each
  in declaration order. In C++ it was the host's permanent automation index
  wherever a format keyed parameters by index: inserting a parameter earlier
  shifted every later index. In Rust it is only the order hosts list parameters
  in; the host's id is a hash of the key (D31), so inserting, reordering or
  regrouping parameters keeps automation.
- Declaring validates everything before anything changes; a refused declaration
  leaves the set as it was. Declaration closes once.
- At most one parameter is the bypass, and it is saved like any other.

**Why.** A rename must not orphan sessions. A duplicate key once destroyed the
live original, a use-after-free in release builds. A marked bypass used to reset
on reload.

**Rust.** A builder that freezes into the set; duplicates and bad keys are `Err`
in every build; the 31-character check is an error, not a debug warning, and
counts bytes. The bypass must be a bool, since VST3 and CLAP drive it as a
toggle. The host id is `host_id(key)` (D56), and a key whose id is reserved or
already taken is `Err` too. A continuous range is checked when it is made, so
the builder never sees a bad one (D69).

Pinned by: `parameter_set_test` "ParameterSet refuses corrupting registrations
in every build", "ParameterSet entries precede subgroups in each group";
`parameter_state_test` "ParameterState flags ids colliding after AAX truncation
in both modes".

### P11. Gestures are counted, and collapse

**Invariant.** A gesture is a per-parameter count on the UI thread. The host
hears one begin when the count goes 0 → 1 and one end when it returns to 0,
however many controls, surfaces, nested brackets or undo replays overlap. Begin
comes before the gesture's first value and end after its last. Each opener holds
a single-use token; dropping it closes it, moving it transfers it, and a token
that outlives its parameter closes as a no-op. A lone edit is a complete
one-shot gesture. Only explicit gestures open one; host, restore and learned
writes never do. A missing end holds the parameter touched until the token
drops; there is no timeout.

**Why.** Hosts need balanced, non-duplicated touch brackets for their automation
write modes. One drag is one undo step. Tokens rather than begin/end calls mean
an error cannot strand the count (`dd575dd`, which introduced them).

**Rust.** A `#[must_use]`, `!Send` token; `Drop` closes it and must not panic,
since it runs during unwinding. `#[must_use]` because a discarded token closes
at once and leaves the write gesture-less, so undo never records it (`90489e5`).
Decide "projected to a host" once, at the edge that opens the gesture, and
remember it for the close: C++ checked it separately at begin and at end, which
could pair them wrongly. Consider a diagnostic for a token held implausibly
long. A panic in the exporter's call to the host leaves what it had not yet told
for the next drain, so the host still hears balanced brackets (D76).

Pinned by: `gesture_test` "nested gestures collapse to one (ref-counted depth)",
"a token outliving its parameter is a safe no-op"; `host_parameter_test` "a
gesture on an attached HostParameter brackets host begin/end exactly 1:1".

### P12. An open gesture holds back host-shaped writes from surfaces

**Invariant.** While any gesture is open on a parameter, the store and the DSP
take host, restore and learned writes, and the plugin's syncs (D34), at once,
but surfaces do not show them and do not mark them seen. Every surface, not only
the one being dragged, shows the user's values as they arrive. The first poll
after the gesture closes shows the store.

**Why.** Automation must not yank a live drag nor leak into its undo step
(`409e9b8`). Consuming the pending write without showing it would lose it for
good; a test pins that the deferral does not (`24845ed`). Other surfaces follow
the drag because a user edit is announced at once whatever the gesture state;
only host-shaped writes and syncs wait. One C++ edge: a surface built while a
gesture was open seeded from the store (`seam_Binding.h:103`), so it showed a
host write the other surfaces were holding back.

**Rust.** The parameter's gesture state keeps the value surfaces show, per
parameter rather than per token, since tokens nest: the store's at the open,
then each user, program or replay write's. While the gesture count is above zero
a watch's poll shows that value and leaves the store's counter unacknowledged; a
watch made then seeds from it and acknowledges nothing. The first poll after the
close reads the store.

Pinned by: `reconcile_test` "a host write deferred by an open gesture projects
through the pump after the close"; `midi_membrane_test` "midi: a write arriving
mid-drag moves the DSP but waits for the widget".

### P13. How a change reaches a surface

**Invariant.** In C++, a change was announced synchronously, in subscription
order, to the subscribers live when the announcement began; one that joined
mid-announcement waited for the next, one that left was never called again, and
after an unsubscribe returned the callback never ran again. In Rust the same
guarantees hold by polling: a surface reads the change counter, and if it moved,
reads the value and shows it if it differs from what it shows. A surface sees
every change made after it started watching, latest value wins, one update per
poll however many writes landed, never a value older than one it already showed.

**Why.** No per-sample fan-out to the UI from audio-rate automation. A surface
that missed a change stayed wrong until the next one.

**Rust.** Host, restore and learned writes are visible to every poller, editor
open or not; C++ made them invisible to observers until some widget was bound.
Where a Rust crate keeps an internal callback list, the C++ rules apply to it:
take the list out before running callbacks (`mem::take`), drop old handles only
after the container is consistent again (commit `d76ff17`, "release old
connections last"), and give a callback a way to remove itself instead of
capturing its own handle.

### P14. Subscribe, then seed

**Invariant.** A new surface starts watching before it reads the current value,
so every write is either in its first read or arrives after it. The first read
never cancels a change already on its way to the surface, and never feeds
deduplication state another surface relies on.

**Why.** A read taken before subscribing dropped a worker write that landed in
between (`7de5df4`). A seed that was queued like a change overwrote a newer
queued change and fed the shared dedup tracker, which then suppressed a pending
host write on a sibling widget (`2298dd5`, `0074684`).

**Rust.** Load the counter (`Acquire`) before the value and record it as
last-seen. A write landing between the two bumps the counter past last-seen, so
the next poll re-reads. No staging, no re-read loop. On the page the same rule
is `ready` after the listener is installed (B1).

Pinned by: `binding_marshal_test` "a write racing any SliderBinding constructor
read still reaches the slider"; `reconcile_test` "a binding built on a hosted
widget leaves a pending host write for its sibling".

### P15. Every surface keeps its own position

**Invariant.** Each surface tracks what it has seen independently. Destroying,
moving or adding one surface never changes what another surface is shown. If any
registry of "someone is watching this parameter" exists, it counts registrations
per parameter: added on 0 → 1, removed on 1 → 0, never a set.

**Why.** The C++ reconcile pump registered a value once and removed it by
address. With two widgets on one parameter under one UI root, destroying either
deregistered both, and the survivor stopped following host automation, restores
and learned controllers until it was rebuilt (`seam_ReconcilePump.h:52`, `:71`;
no test destroys one of two bindings). The web side escaped for two reasons: a
second bind of one parameter got the first attachment back rather than a new
one, and attachments had no detach, living until the host was destroyed.

**Rust.** A per-surface last-seen counter makes the bug impossible: there is no
shared registration and no shared dedup tracker. A test binds two surfaces to
one parameter, drops one, and checks the other still follows host writes.

### P16. A surface shows the store, not the announcement

**Invariant.** After any change, a surface ends on the value the store holds.

**Why.** C++ widget bindings, the page attachments and the coalesced and
debounced observers kept the value that was announced. When an earlier listener
corrected the value inside the announcement (a clamp), or two writers raced, a
widget ended on a value the store no longer held until the next change. Undo's
plain-value policy (`f88b359`) and the web persistence mirror (`27e0311`) had
been fixed to re-read the store; parameter enrollment kept the announced value
by design (`seam_UndoRecorder.h:778`, `:1207-1211`), and the bindings never
changed. Found in this transfer; traced, untested.

**Rust.** Polling reads the store, so this holds by construction. Keep a test
anyway: two writers racing, then a poll, shows the stored value.

### P17. Host-facing metadata

**Invariant.**

- **Steps**: the number of reachable values, an off-lattice end included,
  saturating rather than overflowing; 0 means continuous. Bool has 2; a choice
  has its count.
- **Discrete**: bool and choice are; a stepped continuous parameter is not, so
  every host sees it as continuous (D12). Only bool is boolean.
- **Text**: formatted in real units. A continuous value shows 2 decimals when
  continuous, and otherwise as many as its start or its step needs, the larger:
  each needs the places it was written with, as the lattice reads them (D57), at
  most 6; trailing zeros are trimmed. On a step of 0.125, 0.125 shows as itself,
  where counting from the step's first significant digit would show it as
  "0.12". The lattice counts in the finer of the two (D57), so a whole step from
  0.5 shows its halves, where the step's decimals alone would show 1.5 as "2",
  which reads back as 2.5. An off-lattice end shows the shortest text that reads
  back as itself, so the end stays reachable from a value box (P2). Parsing
  reads a raw number and does not snap (the write snaps). Bool shows On/Off and
  parses on, true or 1. A choice shows its label; an unknown label parses to the
  current index, so a renamed label never jumps to the first choice. A label
  reads in any case, so a declaration whose labels are empty or differ only in
  case is refused: parsing would pick the first of two, and no text reads as an
  empty one. Parsing trims the typed text, so a label with whitespace at its
  start or end is refused too, since no text reads as it either.
- A text request with a maximum length of 0 or less means unlimited.

**Why.** Hosts build automation grids from the step count; value boxes must
round-trip without drift; AU passes a maximum length of 0 (`5c9141f`).

What each host got from C++, through JUCE 9.0.0: VST3 a step count and AU an
integer range only for a discrete parameter
(`juce_audio_plugin_client_VST3.cpp:942-951`,
`juce_audio_plugin_client_AU_1.mm:1046`), so a stepped continuous range reached
both as continuous; AAX got every parameter's count, capped at 2048, with a
continuous type for a stepped range (`juce_audio_plugin_client_AAX.cpp:1720`,
`:1777-1785`). For a whole-number step, C++ passed 0 decimals to JUCE's number
formatting, which then applies no fixed precision: the stream's default of six
significant digits, scientific from 1e6, so an off-lattice end like 10.5 showed
as "10.5" (`seam_Parameter.h:53`, `:103`; `juce_String.cpp:486-489`).

**Rust.** Each exporter maps steps to its format for bool and choice only (VST3
`stepCount = steps - 1`, CLAP's stepped flag); a stepped continuous range is
continuous to every host (D12), and the JUCE adapter is to report it as not
discrete with its real count, as C++ did. That half is not built: the C
interface passes no count for a continuous range, so the adapter reports JUCE's
default. Use distinct names for real-unit and 0..1 parsing: C++ fused two
hierarchies with identical method signatures, so a hosted parameter answered the
wrong step count through its base type.

[History](decision-history.md#p17-host-facing-metadata)

### P18. Coalesced and debounced observation

**Invariant.** A coalesced observer hears at most once per UI tick, with the
latest value, not on subscribe, and stops when dropped. A debounced observer
hears once, with the latest value, after a full quiet window measured from when
the UI thread saw the last change: trailing edge only, no maximum wait,
cancelled on drop.

**Why.** Expensive reactions (a redraw, a save) to a value that streams.

**Rust.** Coalescing is what polling does. A debouncer is a small poll-side
helper; build it when a consumer needs one.

---

## 2. Undo (seam-undo)

The promise: host automation never pollutes undo, and one user action is one
step.

### U1. Enrollment by kind

**Invariant.** A value enters undo only by explicit enrollment, under a policy
fixed by its kind and never overridden. A parameter records only gestures. A
plain value records every committed change. An owned list records its own verbs.
Enrolling a value twice is refused in every build. Enrollment never changes the
value's own behaviour (D11). C++ had one exception: on an enrolled parameter
whose store a restore or host write had moved during a drag, recording the step
wrote the drag's value back (U11).

**Why.** A parameter enrolled under the plain policy would record the host's
writes: C++ first guarded this in debug (`2e465f4`), then refused it in every
build (`f88b359`). Double enrollment was refused in every build for lists but
only in debug for scalars and passive collections (`seam_UndoRecorder.h:1709`,
`:1327-1329`, lists at `:1357`), so release recorded two steps per edit.

**Rust.** Separate handle types per kind, so a parameter cannot reach the plain
policy; `HistoryError::AlreadyEnrolled` for a second enrollment. There is one
`History` per `UiParams` (D40), so no parameter is enrolled in two.

### U2. Enrollment records nothing

**Invariant.** Enrolling captures the current value as the baseline and records
no step. Nothing written before enrollment, or while not enrolled, is ever
recorded, except within a gesture already open at enrollment (U3). That
exception is D2's design: C++ recorded only what changed after the enrollment.

**Why.** A before-state the value never held makes undo write a value the user
never chose. C++ took the baseline from the live value from its first recorder
commit (`a16220c`).

### U3. Enrolling during an open gesture

**Invariant.** If a gesture is already open when a value is enrolled, the
enrollment treats it as open: writes until the close are tracked, not each
recorded, and the close records at most one step. A close with no open, seen or
seeded at enrollment, records nothing. The before-state is the value at
enrollment in C++ (tests pin "not the pre-drag value"); in Rust it is the value
at the gesture's open (D2).

**Why.** The open is announced only on the 0 → 1 edge, so a mid-drag enrollment
saw only the close and recorded a before-state of zero; undo then wrote a value
never held, through a full host gesture (`26f07ce`).

**Rust.** With D2, the gesture record carries its open value and the question
disappears.

Pinned by: `undo_recorder_test` "record() mid-gesture seeds the before-state
from the live value", "record() mid-gesture on a parameter seeds the
before-state from the live value" (its "not the pre-drag 2.0f"), "record()
mid-gesture with no further movement records nothing at the close".

### U4. A gesture is one step

**Invariant.** All writes inside one gesture produce at most one step: from the
value at open to the last value the user wrote before the close, recorded at the
close only if they differ. The after-value is never read from a cell the host
also writes.

**Why.** A drag is one undo step. Reading the after-value from the shared store
let a host write during the drag become the step (`4d83b5b`).

**Rust.** The parameter's gesture state records the user's last value (P12); the
outermost close emits a `GestureRecord` with both, unless a replay opened the
gesture (U9).

Pinned by: `undo_host_param_test` "host automation DURING a gesture cannot taint
the recorded step"; `undo_recorder_test` "a gesture that does not move the value
records nothing".

### U5. Writes outside a gesture

**Invariant.** Outside any gesture, a plain value records each committed change
as its own step, with the value it settled on (after any correction), and a
parameter records nothing. Either way the baseline advances to the newest value.

**Why.** Plain values have no host to protect. Recording the announced value
instead of the settled one made one corrected write into two steps, so the first
undo was a visible no-op (`f88b359`). The C++ fix and its test cover a
correcting listener connected before the enrollment
(`seam_UndoRecorder.h:770-778`; `undo_recorder_test` "one user set() that an
earlier clamp listener corrects is one undo step back to the start").

**Rust.** Constraints live in the write path, not in listeners, so the settled
value is simply what the write stored. Undo polls once per UI tick, so a
committed change is what one tick sees: a plain value's writes between two ticks
coalesce into one step, latest value wins, whichever thread made them (U14).
Undo, redo and jump tick first, so they undo a change made since the last tick.
Commits queue until a tick takes them, so the plugin ticks the history on every
UI tick, editor open or not. Within one tick, steps record in the order their
changes happened (D36).

### U6. Transactions

**Invariant.** While a transaction is open, every recorded step joins one
history entry named by the outermost transaction; nested transactions join it;
an empty one leaves no entry; one that ends in an error keeps what it recorded
as one entry. Undo or redo inside an open transaction ends it, and later steps
form a new entry, which keeps the name of the transaction still open.

**Why.** One user operation that touches several values ("drop a sample") is one
step (`839facb`). In C++ the new entry had no name: JUCE's undo and redo begin
an unnamed transaction, and a step inside an open scope did not name it
(`seam_UndoRecorder.h:1553-1555`). The name of the operation the user is still
inside is the useful one.

**Rust.** The open-transaction depth lives on the history, not per recorder: C++
kept it per recorder and the grouping per shared history, so a second recorder
split another's transaction. A guard closes the scope on drop.

### U7. No-op writes record nothing, by a reflexive equality

**Invariant.** A write that leaves the value equal records nothing and keeps the
redo stack. Equality for this decision is reflexive: a value equals a copy of
itself, NaN payloads included.

**Why.** A no-op command after an undo must not destroy the redo. A NaN inside a
collection under `==` made it re-record and re-replay itself forever on the UI
thread (`be2d94d`).

**Rust.** An `UndoEq` trait, required by enrollment, so every enrolled type
states its equality. The standard numbers, strings, tuples, options, arrays and
collections implement it, and two macros write it for a plugin's own type with
its `UndoSize` (D8): `undo_by_eq!` from the type's `==`, and `undo_by_fields!`
field by field. The compiler checks that a type states one, not that it is
reflexive: `undo_by_eq!` takes the type's word that its `==` is. `undo_by_eq!`
also requires `Copy`, since it books a value at its size and a `Copy` type owns
nothing on the heap. For floats it keeps C++'s rule: equal when bit-identical or
`==`, so a NaN equals itself and −0 equals +0, and a write that only flips a
zero's sign keeps the redo. C++ guaranteed this only for trivially copyable
payloads (`seam_UndoRecorder.h:62-73`); `UndoEq` makes every enrolled type state
it.

### U8. The baseline follows replay

**Invariant.** After an undo or redo, the next real change records the value the
replay left as its before-state.

**Why.** Edit, undo, edit recorded a stale before-state; undoing the second edit
landed on a state that never existed (`4d83b5b`).

### U9. A replay never records itself

**Invariant.** Applying a step never records a step for the value it drives.
History size and cursor move only by the undo or redo itself.

**Why.** A re-recorded replay inserts a phantom step whose push destroys the
redo (`9242759`).

**Rust.** The replay writes with `Origin::Replay`, which updates baselines and
bumps change counters but never records. A gesture a replay opens emits no
`GestureRecord` at its close, since nothing else would stop it recording a
phantom step that destroys the redo. In C++ the enrollment ignored its own
replay's gesture (`seam_UndoRecorder.h:814`), but during an undo or redo a
second guard already refused to record (`:880-881`, since `822d806`). Line 814
mattered for the write-back at record time (U11), whose gesture would otherwise
have recorded a step inside the one being recorded. C++ needed a per-value flag
that relied on synchronous delivery, plus a first-in-first-out list of exact
targets for collections whose writes were deferred; both are moot.

Pinned by: `undo_recorder_test` "replay drives observers through set() without
re-recording"; `undo_host_param_test` "parameter replay brackets a complete host
gesture".

### U10. Writes caused by a replay

**Invariant.** A write caused by a replay (a derived value updated from the
replayed one) lands but records nothing, and its baseline advances.

**Why.** Effects re-run on undo, but they are not new user actions. Recording
them mid-replay broke every debug undo over the derived-value pattern
(`822d806`).

**Rust.** Each change carries the origin of its last writer beside its change
counter, for parameters, plain values and lists alike (D38). A derivation reads
the origin of the change it derives from and passes it on, through
`Value::set_from` or `List::derived`, and never records, whatever that origin is
(D43): its source's step covers it, and a replay of the source derives it again.
C++ never fixed the deferred case: `9242759` stopped a collection re-recording
its own deferred replay, but a write cascaded to another value from that
deferred drain still recorded a new step and wiped the redo
(`undo_reentrancy_test` "a cross-value write cascaded from a deferred replay
drain records as a new step"; `seam_UndoList.h:50-60`). C++ dropped a list verb
issued during a replay entirely while letting a scalar write land; Rust uses the
one rule above for every kind. A list verb that records nothing still lands, and
makes the list's earlier steps replay as nothing, since they index into contents
it changed.

### U11. What a replay does

**Invariant.** Undo and redo apply values through the normal write path, so
every surface follows and effects attached to the value re-run. A parameter
replay that changes the value is exactly one complete host gesture. A replay
that would change nothing does nothing. Recording a step writes nothing (D11).

**Why.** Strict hosts need a complete touch. Applying at record time fired an
empty host begin and end at every drag end and disrupted live slider drags
(`11ff831`).

In C++, recording did apply the step whenever the store had moved from its
after-value: JUCE's undo manager performs an action as it records it, and the
step's perform skipped only an equal value (`seam_UndoRecorder.h:146-149`,
`:172`). That happened on an enrolled parameter whose drag outlived a restore or
a host write, and wrote the drag's last value over it as one extra host gesture.
Rust does not carry it over (D11). The C++ form is pinned by
`state_persistence_test` "a mid-gesture restore: the drag wins, and its close
may fire one host gesture (enrolled) or none (not enrolled)" and
`web_parameter_attachment_test` "web bridge: a host write mid web-drag is
deferred until the drag ends".

### U12. Replays are atomic (new in Rust)

**Invariant.** While a replay is applying, undo, redo, jump, clear and record
requests are refused. A replay either completes and moves the cursor, or fails
and leaves a defined state.

**Why.** C++ left it undefined. A nested undo re-undid the same entry and moved
the cursor twice, marking an entry undone that was never replayed; an error
mid-replay left values and history disagreeing.

**Rust.** A history state machine (`Idle`, `Replaying`) returning
`HistoryError::Replaying`. Take the steps out, apply them with nothing of the
history borrowed, then commit. The only lock a replay takes of its own is D39's
load hold, around a parameter write. A panic during the apply leaves the machine
recoverable.

### U13. Stale steps replay as no-ops

**Invariant.** A step whose value was forgotten, dropped or reset stays in the
history but replays as a no-op. Values, lists and the history can be dropped in
any order, which is new in Rust: C++ required a parameter, value or collection
to outlive the recorder or be forgotten (`seam_UndoRecorder.h:1213`, `:1236`,
`:1311`), and only a list forgot itself (`seam_UndoList.h:86-90`). Each element
a step holds is freed exactly once.

**Why.** History outlives enrollments; a replay against a dead value is a
use-after-free (`a16220c`, `4adccfb`).

**Rust.** Steps hold `Weak` handles or generation-checked ids, and the history
holds enrolled values and lists weakly, forgetting one at the tick after its
last handle drops (D37). Consider showing stale entries greyed out rather than
as silent no-ops.

### U14. One history thread

**Invariant.** The history changes on the UI thread only. A burst of writes to a
plain value from another thread becomes one step per poll that sees it, latest
value wins.

**Why.** The history is a UI-thread object. C++'s undo manager was
message-thread-only, so the recorder marshalled writes from other threads onto
that thread, latest value wins (`a16220c`).

Pinned by: `undo_recorder_test` "an off-thread set() burst coalesces to one
recorded step".

### U15. Host exclusion

**Invariant.** Host automation, host restores and gesture-less parameter writes
never create, change or extend a step. A host write during a user's gesture
never becomes that step's before- or after-value, beyond being the value at the
open, and the gesture's close leaves it standing (D11). The host path has no way
to open a gesture.

**Why.** It is the product promise. In C++ it was structural: automation had no
path to the gesture signal. An enrolled parameter's close did write the drag's
value over a host write that landed during the drag (U11); an unenrolled one
left the host's value standing (`web_parameter_attachment_test` "web bridge:
without undo enrollment the deferred host write lands at drag end").

**Rust.** The top test priority: a property test interleaving random host
writes, program writes and gestures (property 6 in section 7).

### U16. Restores are not edits

**Invariant.** Writes made while restoring (a full restore, or a late
participant's apply) record nothing, and baselines follow the restored values. A
plain value whose gesture is open across a restore records from the restored
value. A parameter's open gesture keeps its pre-drag before-state and records
the drag's last value, but its close writes nothing, so the restored value
stands (D11); undoing that step writes the pre-drag value, which predates the
restore. C++ called this "the drag wins": on an enrolled parameter the close
wrote the drag's value back over the restore (U11). History recorded before a
restore is cleared once the restore has applied; a late participant's apply
clears nothing; a restore that lands during a replay cannot clear mid-walk.

**Why.** Pre-restore steps replay over unrelated state: by index for lists, as
host gestures for parameters (`f88b359`, `6b9db53`).

**Rust.** Parameter restores write with `Origin::Restore`. For participants, the
session takes the applying mark for the plugin's `RestoreClock` (D19) on the UI
thread around each `apply`, and a plain value or list reads that thread's mark
when it records: no participant threads an origin through its setters, there is
no broadcast window to miss, and a worker thread's write during the apply
records as the worker's. A participant writes parameters through a sync, which
records nothing and tells the host no gesture (D49). The history reads the
clock's two counts on every operation (record, undo, redo, jump, listing) and
clears when either has moved (D7, D20). Under D13 the apply runs on a later tick
than the store, so the stored count is what keeps an undo pressed between the
two from replaying pre-restore steps; the completed count then drops any step
recorded in that window, which the pending restore outranks (S8). A load on
another thread cannot land between a replay step's check of the stored count and
its write (D39). No option keeps history across a restore (D7): list steps would
replay by index over new contents, and parameter steps over the restored values.

Pinned by: `undo_session_test` "a recorder watching the session records nothing
a restore writes", "a late registration's apply records nothing and clears
nothing", "clearHistory keeps an old list step from replaying by index over the
restored list".

### U17. Lists

**Invariant.** Each list verb (insert, remove, move, clear) is one step, or
joins the open transaction. Undo and redo restore elements by identity, not by
copy. Every element has exactly one owner at a time: the list while present, the
step while removed. An invalid or no-op verb (a move onto its own slot, clearing
an empty list) records nothing. A wholesale reset (loading) records nothing and
makes every earlier step inert; steps recorded after it replay as usual. A stale
index-based step replays as a detectable no-op.

**Why.** Move-only elements cannot round-trip through copies (`d78c6dc`,
`4adccfb`). Replays indexing past the end of a list shrunk out of band were
undefined behaviour (`7d1a44e`). The C++ list verbs restored by identity, but
`UndoList::update`, an arbitrary edit, recorded and restored whole-vector copies
(`seam_UndoList.h:280-299`).

**Rust.** `Option<T>` take and put inside the step gives single ownership. Lists
are the only collection kind; the C++ passive whole-collection snapshot existed
to retrofit containers whose type could not change. Give the clear step the same
staleness guard as the others (C++ did not). A verb made while a restore
applies, or any verb through `List::derived` (D43), goes stale like a reset. The
list hands each batch of verbs to undo with the generation they were made at,
checked under the list's lock at each replay. A step that finds the list holding
a verb the history has not taken yet, one another thread made after the tick
began (D42's cut leaves those waiting), drops that verb and goes stale with
every earlier step of the list (D41). A replay of a plain value drops its
waiting commits the same way, so another thread's write the replay wrote over
never records on top of it.

### U18. The history listing

**Invariant.** The history reads as named entries: applied ones oldest first,
then undone ones in redo order. The applied entries are always a prefix and the
cursor is their count. An entry's name is its outermost transaction's; a step
outside any transaction takes the enrolled value's name; the entry steps join
after an undo or redo inside an open transaction takes that transaction's name
(U6). The listing's counter moves only when the listing visibly changes. Jumping
to index k leaves exactly k entries applied, through ordinary undo and redo
hops, seen by pollers as one change; out of range changes nothing.

**Why.** A history panel in two calls (`d2ddb1b`). C++ also named a natively
performed action by the name it was given, and left an entry opened without a
name empty, as after an undo inside a transaction (`undo_history_test` "history
names: nested transact is one entry, performed and enrolled steps keep their
names" expects `{Outer, Native, "", A}`).

### U19. A bounded history

**Invariant.** History is bounded: when the booked size of the steps passes a
soft cap, the oldest applied entries are dropped whole, never below a floor.
Each step books its size once, when recorded.

**Why.** Bounded memory. A size that changed after recording made the ledger
drift (`4da51a9`).

C++ booked a step as its action's size plus its two payloads, which the action's
size already holds, so each payload counted twice; heap a payload owned was not
counted (`seam_UndoRecorder.h:163`). The floor counted transactions, which are
Rust's entries, not steps (`juce_UndoManager.cpp:204-206`). The recorder's
comment called its defaults JUCE's own, "unbounded history, no transaction
floor", and said 0 meant no limit; JUCE's defaults are 30,000 and 30, and it
raises 0 to 1 (`seam_UndoRecorder.h:1154-1158`, `juce_UndoManager.cpp:102`).

**Rust.** D8: 30,000 units, a step booked at the bytes its stored values keep
alive (`UndoSize`, so a vector's heap counts) plus 32, and a floor of 30
entries, documented as what they are. Pruning drops whole entries, never single
steps out of one. A test records real edits and checks the entries kept against
the documented numbers, so a booking that drifts fails it.

[History](decision-history.md#u19-a-bounded-history)

---

## 3. Session (seam-session)

### S1. One section per owner

**Invariant.** Each owner of saved state other than parameters saves one section
of the `extra` object under a stable, unique, non-empty key. Nothing to save
leaves the key out, and an empty `extra` is left out of the block.

### S2. The session does not own participants

**Invariant.** A registration is a handle; the session and the handle may end in
either order. A participant that leaves during a pass is skipped. An apply whose
session is gone does nothing.

### S3. Threads

**Invariant.** Save and restore run on any thread but the audio thread.
Participant code runs on the UI thread only. No lock is held while participant
code runs, a restore never waits for the UI thread, and the audio thread never
touches the session.

**Why.** Hosts save and restore from their own threads; participants are UI
objects.

### S4. Save reads snapshots

**Invariant.** A save never runs participant code. It writes each participant's
last capture (taken at registration, on change, or after an apply), in
registration order, then the sections nobody holds.

**Why.** A save that read live UI state from a host thread raced it: a data race
on UI state, which the C++ header warned "fails silently rather than loudly"
(`git show f88b359^:modules/seam_plugin/seam_PluginProcessor.h`, lines 164-168).

**Rust.** `Session::save` inserts the sections in key order, as learn's section
and the saved block's kept keys do, so the bytes are the same whatever order
participants registered in and whether or not some crate in the build turns on
`serde_json`'s `preserve_order`. A capture nested deeper than 125 levels, or
one that would grow the block past 64 MiB, is refused as it is taken and the
previous one stays saved, so a save never writes a block the same build cannot
load (D77).

### S5. Store now, apply later

**Invariant.** A restore stores every section at once, atomically across
sections, so a save straight afterwards returns exactly what was restored,
before any participant has applied. Participants apply on a later UI tick,
whatever thread the restore came from (D13). Restores that arrive before that
apply collapse into one apply of the newest.

**Why.** A host that saves right after loading must get the loaded state. In C++
that described an off-thread restore only: a restore on the message thread with
no pass running applied inline, before `restore` returned
(`seam_SessionRegistry.h:755-756`), so a save straight afterwards already held
each participant's re-capture.

**Rust.** An owed flag the UI tick drains replaces the C++ message hop. Since
every apply now waits for a tick, an undo between the store and the pass must
not replay pre-restore steps: the restore bumps the clock's stored count as it
stores (U16, D20).

Pinned by: `session_registry_test` "an off-thread restore stores the raw
sections at once and applies them on the message thread", "restores before the
hop coalesce: the newest section applies once, the hook runs once". The C++
inline case, which D13 drops: "a restore on the message thread applies inline,
before it returns".

### S6. The apply pass

**Invariant.** A pass applies each pending section in registration order (over
the participants registered when it began), re-capturing each participant after
its apply, then marks the restore completed once. It runs even with no
participants, so a plugin with only parameters can still clear its undo history.
Passes never nest: a restore that arrives during a pass gets its own later pass.

**Why.** A nested pass would apply an older section after a newer one.

**Rust.** "Completed" is the `RestoreClock`'s completed count (D7, D19), not a
callback.

### S7. Complete assignment, contained failures

**Invariant.** A restore assigns every registered participant: a missing key, a
missing or non-object `extra`, or a JSON `null` section means "reset to
defaults". A participant whose apply fails, for any reason, keeps its state, the
pass continues, and its raw section is kept unchanged for the next save (D14).
Nothing reaches the host.

**Why.** A restore must not depend on prior state (the parameters' rule too).
One bad section must not stop its siblings, cross the host boundary, or be
overwritten on the next save by pre-restore state; a newer build may have
written it.

C++ kept the raw section only when the apply, or the capture after it, threw
(`seam_SessionRegistry.h:449-459`, `:472`, `:481`). Its contract (`:114-115`)
and its shipped persistence participant (`seam_PersistenceRegistry.h:497-498`)
reset to defaults on data that did not decode, and the next save wrote the
defaults over it (`state_persistence_test` "an undecodable persisted entry
defaults that entry; setStateInformation does not throw"). Keeping the raw
section on any failure is the change (D14).

A JSON `null` section under a key no participant held was kept, claimed by the
next participant to register that key, and applied as "no section", which reset
it; an absent key left that participant alone (`seam_SessionRegistry.h:200-202`,
`:222`, `:237-247`; traced, untested). With `null` meaning absent, both cases
take D6's answer.

**Rust.** A panicking apply is contained as a failed one is, and the participant
is held: its raw section stays saved and its captures are ignored until a later
apply succeeds, since a panic can stop an apply halfway (D55).

### S8. A pending restore outranks live edits

**Invariant.** Between a restore being stored and its apply committing, a
participant's saved section is the restored one. A change notification from that
participant does nothing, and a capture in progress when the restore landed is
discarded.

**Why.** Otherwise an edit between an off-thread restore and its deferred apply
overwrote the restored section in saves, and a reload lost the restore.

**Rust.** One session-wide restore counter, the stamp (S9), read when an apply
or a capture starts. C++ guarded the capture with a boolean, which a restore
stored and applied during the capture slipped past. That needs the capture
itself to pump the message loop or call `restore` on the message thread, since
an apply runs only there and never nests (`seam_SessionRegistry.h:181-191`,
`:331-345`, `:755`).

### S9. The stale-apply stamp

**Invariant.** Every restore takes the next number from a counter that only
grows and stamps every section it stores. An apply reads the section and its
stamp, runs the participant without any lock, then commits (clear pending; saved
section becomes the re-capture, or the raw section if the apply failed) only if
the section still carries the same stamp. Otherwise a newer restore was stored
while the participant ran: its section stays pending and a later pass applies
it.

**Why.** Without it, an older apply's commit cleared the newer restore's pending
mark and saved its re-capture of the older state, so the newer restore never
applied. The newer restore can come from a host thread or from inside the apply,
and the older apply may itself be a late one (S10). A late registration never
stamps another participant's section: it stamps only the section it claims, with
the newest restore's number (`seam_SessionRegistry.h:232-254`), and a section
nobody holds carries no stamp until then (`:245`). Coalescing (S5) handles older
deferred applies; S8 handles user edits.

**Rust.** The stamp is the session's restore counter, read when an apply or a
capture starts; the commit, under the session's lock, does nothing if it moved
or a newer section is pending.

Pinned by: `session_registry_test` "a restore landing during an apply keeps its
own section and applies after" (removing the stamp check fails it), "off-thread
restores race releases, claims and late applies without losing or regressing the
section".

### S10. Late participants

**Invariant.** A section waiting under a key (restored before any participant
had it, or kept from a registration that ended) is claimed when a participant
registers that exact key, atomically with the registration, and applied before
registration returns, then re-captured. A late apply is restore-neutral for undo
(S12) but is not a restore: it marks no restore completed and clears no history.
A failed late apply keeps the raw section and the registration still succeeds. A
pass from the same restore never applies it a second time. With nothing waiting,
nothing is applied (D6 changes this to "reset to defaults" when the latest
restore lacked the key).

**Why.** Before `6b9db53`, a section for an absent participant was dropped, so
an editor opened after the host's restore lost its state. Completion stays off a
late apply because a registration can come minutes after the restore, and
clearing then would wipe the user's undo history.

**Rust.** The C++ rule "register last, in the most-derived constructor" is moot:
the participant is fully built before it registers.

Pinned by: `session_registry_test` "a participant registering after a restore
applies its restored section, then captures it"; `plugin_processor_test` "an
editor's session state is restored when it opens after the host restore, and
saved after it closes".

### S11. Kept sections

**Invariant.** When a registration ends, its last section stays in the session
unless it is null, "nothing to save" (an empty object stays: C++ tested for
void, not for empty, `seam_SessionRegistry.h:268`), is written by every save,
and is applied to the next participant that registers the key, over whatever
that object holds. A restore while the key is unheld replaces or drops it.
Sections no participant ever claims (from a newer build) are saved back
verbatim. A key is held or kept, never both, and hand-over is atomic: a
concurrent save sees a section exactly once. To forget a section: capture
nothing, mark changed, then end the registration, once no restore is pending for
it; while a restored section waits, the change does nothing (S8) and ending the
registration keeps the restored section (`seam_SessionRegistry.h:626-629`).

**Why.** An editor's layout must survive the editor closing before the host
saves. A session saved by a richer build must not lose data in a poorer one.

### S12. What a restore does to undo

**Invariant.** Writes made while applying restored state, in a pass or a late
apply, never become undo steps, and undo baselines follow them. History from
before a restore (not a late apply) is cleared once it has applied.

**Why.** In C++ this was opt-in: a recorder kept restores out only after
`ignoreRestores` (`undo_session_test` "without ignoreRestores the same restore
records its writes"). In Rust every history does.

**Rust.** The applying thread's mark for the shared `RestoreClock`, and the
clock's stored and completed counts (D7, D19, D20, U16). The C++ window signal's
side effects disappear with it: an unrelated open drag on a plain value rebased
at the window's close (parameters were exempt, `seam_UndoRecorder.h:800`,
enrolled at `:1218`), and a recorder connected, in signal order, after an open
listener that registers a participant recorded that participant's apply.

### S13. Parameters first

**Invariant.** On load, parameters are assigned first and synchronously; the
session then receives `extra`, so a participant's apply sees restored parameter
values. A block that is not recognised writes nothing at all, parameters or
session.

**Why.** Participant state may derive from parameters. The C++ design spec
weighed applying parameters later on the UI thread and rejected it: the audio
thread would render pre-restore values and an immediate save would capture old
state (`docs/superpowers/specs/2026-07-30-json-state-persistence.md:576-580` in
seam-cpp).

Pinned by: `plugin_processor_test` "setStateInformation applies the parameters
before any session participant". That an unrecognised block leaves the session
alone is untested in C++; Rust pins it (`crates/host/tests/state.rs`).

---

## 4. MIDI learn (seam-learn)

A user arms a target, moves a controller, and from then on that controller
drives the target. The C++ learn engine had no MIDI 2.0, pitch bend, aftertouch,
RPN, relative-encoder or pickup support; those are design questions (D10, D21,
D22), not contracts.

### L1. Registering a target is not mapping it

**Invariant.** Registering a target creates no mapping, except that a restored
entry held for its id lands then (L9; `seam_MidiLearnEngine.h:154-158`,
`:954-968`). Only mapped targets reach the audio thread's lookup, so its cost
follows the number of mappings, not of targets. A target has at most one source;
a new learn, set or restore replaces it. One source may drive any number of
targets.

Pinned by: `midi_engine_test` "midi: an enrolled but unlearned target
contributes zero dispatch slots", "midi: one source may drive several targets".

### L2. What can be learned

**Invariant.** In C++: 7-bit control changes, and 14-bit NRPNs assembled from
controllers 99, 98, 6 and 38. Those four controllers never act as plain
controllers. Everything else is ignored. Reserved controllers block capture
only; reserved NRPNs block capture and dispatch; the four NRPN controllers block
both. The reservation policy is per instance. Controllers 101 and 100, the RPN
address, were plain learnable controllers and never reset the NRPN address
(`seam_MidiLearnConfig.h:81-84`; engine `:504-507`, `:698-713`).

**Rust.** The input is Universal MIDI Packets (D10 settles the scaling and
native MIDI 2.0 controllers; D21 the group and channel). An NRPN source is
either MIDI 1.0's four-controller NRPN or a MIDI 2.0 assignable controller,
numbered bank × 128 + index, so both bind the same way. MIDI 2.0's control
changes 6, 38 and 98 to 101 mean nothing and are dropped, as JUCE's translator
never emits them; 96 and 97, which it passes through, are dropped too, since a
MIDI 2.0 stream selects no number for them to step. A reserved NRPN is blocked
by refusing any mapping whose group and channel could hear it, so an any-channel
mapping of a number reserved on one channel is refused (D35). The parser keeps
each group and channel's MIDI 1.0 address, data entry and controller pairs as
D70 states. Data increment and decrement (96, 97) belong to the parser too,
unlike C++, and drive nothing, since their value is a step for the selected
number rather than a position, so a binding would jump its target and follow
every RPN's increments; learn does not apply the step, which needs the value the
receiver holds. Reserved controllers block capture only, as in C++: `Config`'s
`reserved_ccs`, and the channel mode messages 120 to 127 unless
`reserve_channel_mode` is turned off, so a panic button pressed while armed is
not learned; a mapping made by hand still dispatches. VST3 delivers controllers
only as parameter changes; the VST3 exporter routes them to learn through hidden
proxy parameters (D22). The parser owns 101 and 100 as well. Fix while porting:
RPN data entry was dispatched as the last NRPN's value; an NRPN sent with only
its high byte topped out at 0.9922, never 1.0 (D30 settles it); the NRPN parser
stopped running while nothing was mapped or armed, so its address went stale.
The Rust crate also learns notes to parameters (`Source::Note`), for instruments
that map keys to parameters, though the C++ Seam never learned notes (D17). A
note-on writes its velocity and a note-off writes nothing; an arm states whether
it takes a note.

Pinned by: `midi_source_test` "midi: a reserved CC range blocks capture only, on
the configured channel", "midi: a reserved NRPN range blocks capture AND
dispatch", "midi: the four parser-owned CCs are blocked in both directions";
`midi_engine_test` "midi: an MSB-only NRPN resolves as msb shifted, and a
partial sequence is dropped".

### L3. The learn flow

**Invariant.** Arming names one target; the last arm wins. The audio thread
stages the newest learnable message without locks. The UI thread's next poll
commits it as the mapping and disarms. A message from a blocked source is
refused and the target stays armed. Cancelling leaves the mappings alone.

**Rust.** Fix while porting: a staged capture carried no arm identity, so a
message staged for one arm could commit to the next. A re-arm leaves the armed
flag raised, and a block that read it before the re-arm still stages afterwards
(`seam_MidiLearnEngine.h:349-352`, `:487`, `:771-781`, `:795-818`); give each
arm a generation the audio thread stages with the message. An edit of the armed
target's mapping did not cancel the arm, so the next message overwrote the
user's edit; a blocked message later in a tick hid an earlier legal one, because
only the newest was kept. An NRPN captured from a high byte alone stays staged,
uncommitted, until a learn poll finds it unchanged since an earlier poll, once a
whole audio block after it has ended (D47), or a new high byte arrives without
one; a low byte for it upgrades the capture to 14-bit (D30). Over DIN, CC 6 and
CC 38 often straddle a block, and a poll between them would learn a 14-bit
sender as 7-bit for good. Rust refuses a blocked message on the audio thread,
before it reaches the staged slot, so it never hides an earlier legal one; a
waiting high byte stages as unsettled, so the poll does not commit an older
message it supersedes either. A MIDI 1.0 controller 32 to 63 that comes right
after the controller 32 below it, on the same group and channel, stages as that
one, the pair's high byte, so a 14-bit knob learns the half that does not wrap
as it turns; one with no high byte just before it stages as waiting, so a high
byte that follows wins (D70). A commit maps the message's own group and channel
through the identity curve, as C++ mapped its channel, and a mapping the ceiling
refuses ends the arm.

Pinned by: `midi_capture_test` "midi: arming captures the next message and
commits it on the poll", "midi: several captures before a poll commit only the
last", "midi: cancelling an arm leaves the table untouched".

### L4. The dispatch snapshot

**Invariant.** The audio thread reads mappings from an immutable snapshot,
sorted for lookup, built on the UI thread from the one authoritative table.
Anyone watching the mappings hears of a change only after its snapshot is
published. The audio thread reads without locks, allocation or reference
counting, and never frees. A replaced snapshot is freed on the UI thread once no
read can still hold it; the UI thread never waits for the audio thread.

The protocol, with C++'s orderings:

| Step | Thread | Operation | C++ ordering | Needed (access form; the fence form below is equivalent) |
| --- | --- | --- | --- | --- |
| Publish | UI | build the new table; retire the old one to a list; store the new pointer | release | **SeqCst** |
| Reclaim | UI | if any table is retired, load the reader count; free every retired table if it is 0 | acquire | **SeqCst** |
| Lease | audio | increment the reader count | acquire | **SeqCst** |
| | audio | load the pointer, then read the table | acquire | **SeqCst** |
| Release | audio | decrement the reader count | release | release |

**Why SeqCst.** The design's argument is "either the reclaim sees the reader's
increment, or the reader sees the new pointer". That is the store-buffering
shape, and acquire and release do not give it: they order only when a load reads
the other side's write, and here both loads can read old values. So the reclaim
can read zero readers while the reader loads the old pointer, and the UI thread
frees a table the audio thread is reading, including a call through a freed
sink. x86 can reorder the publisher's store and load; ARMv8 can reorder the
reader's increment and load (an acquire-only atomic add). Only a single total
order over the four operations, from SeqCst on each or a SeqCst fence between
each pair, forbids it. The release on the decrement and the acquire on the
reclaim's load stay: they make the reader's last access happen before the free.

Verified in `seam_MidiDispatchTable.h:112`, `:125`, `:159`, `:160`. No test or
thread sanitizer can see it, since it needs the reordering to actually happen; a
model checker sees it at once. Practically unreachable on today's hardware,
which is not the bar.

A reclaim can run in the same UI tick as the publish that retired the table (a
learn commit, or an edit from a listener the poll ran), not a tick later.

**Rust.** D9: a `triple_buffer` holds the published table, which meets this
invariant with no `unsafe` in Seam: the audio thread takes the newest table
without waiting, and a replaced table drops on the UI thread at a later publish.
Both hold in `triple_buffer` 9.0.0's source: the reader's `read` is a relaxed
load and, when a table is waiting, one `swap(AcqRel)`, and the writer's `write`
drops the stale table its own buffer held. The writer's publish is a
`swap(AcqRel)` on the same atomic, so the store-buffering shape above cannot
arise. The buffers free when both ends have dropped, so the audio side's end is
dropped off the audio thread. The hand-written form below stays as the fallback,
unused, so neither it nor its models exist in the code.

The fence form, were the fallback needed: publisher `store(new, Release);
fence(SeqCst); readers.load(Acquire)`; reader `readers.fetch_add(1, Relaxed);
fence(SeqCst); published.load(Acquire)`. It would cost one fence per block. It
would be written this way because the `loom` model can check it: loom models a
SeqCst fence strongly enough for this shape but treats SeqCst accesses as
acquire/release, so the access form would fail the model while being correct.
The model would come with the table's first commit: one publish against one
lease, with a `loom::cell::UnsafeCell` payload that the free writes and the
reader reads, so loom flags an unsynchronised free directly; it must fail
without the fences and pass with them. A second model would cover two publishes
before one reclaim, and the loom limitation would be confirmed against the
version pinned then.

Pinned by: `midi_thread_test` "midi: a retired snapshot is not freed while a
reader holds one" (the reader count, not the ordering, which no C++ test can
reach).

[History](decision-history.md#l4-the-dispatch-snapshot)

### L5. The audio entry

**Invariant.** Learn runs once per block over the block's MIDI in order, applies
values at block granularity, and does not filter the MIDI the plugin itself
receives. It allocates nothing, and it never runs concurrently with itself: the
parser and the capture staging have one writer (`seam_MidiLearnEngine.h:56-58`).
In Rust, `&mut self` on the audio entry gives that.

Pinned by: `midi_engine_test` "midi: events apply in buffer order, so the block
ends on the last one"; `perf_alloc_gate_test` "midi: warm processMidi dispatch
is zero-alloc".

**Rust.** A test also pins that every packet reaches the processor untouched and
in order, SysEx7 included, since MIDI-CI rides on it (D24).

### L6. From controller value to target value

**Invariant.** A controller's value becomes 0 to 1 (in C++ `v/127` for a control
change and `v/16383` for an NRPN; Rust scales by D10), then passes the mapping's
curve (clamp, invert, skew, window; the identity curve changes nothing), then
the target's range, where the target's write snaps it (P2). Absolute jump only:
no pickup or takeover.

**Rust.** MIDI 1.0 values normalise centre-anchored and MIDI 2.0 values linearly
(D10), so 0, the centre and the maximum land on 0, 0.5 and 1 at every width.
Validate curves when they are made. C++ never did: a skew of 0 flattened the
curve to a constant, a negative skew gave values above 1 and infinity at 0, and
a window outside 0..1 gave out-of-range values (`seam_MidiSource.h:136-137`).

Pinned by: `midi_source_test` "midi: curve window, skew and invert transform as
documented".

### L7. Which origin a learned write uses

**Invariant.** A host-projected parameter is written on the audio thread in the
block the message arrived in, as a learned write (P8): no gesture, no undo, no
host notification (unless the relay is on, H4); surfaces follow on their next
poll. Any other target is written on the UI thread inside one gesture per burst,
so a sweep is one undo step. A burst opens once and closes once: after an idle
period (15 poll ticks), an unmap, a detach or the engine's end
(`seam_MidiLearnEngine.h:138-145`); a close that arrives inside the open lands
after it. Which tier a target uses is decided once, at registration.

**Why.** Learned control must reach the DSP without a UI round trip. It must add
no undo step on the audio-thread tier, and at most one per sweep on the UI tier
(`midi_membrane_test` "midi: a marshaled target follows one message tick later,
as one undo step").

Pinned by: `midi_membrane_test` "midi: a controller sweep over enrolled
parameters records NOTHING in undo"; `midi_engine_test` "midi: tiers are
resolved once, at enrollment".

**Rust.** Fix while porting: a UI-tier target had no "the user owns it" rule, so
MIDI wrote through a drag, merged into its undo step, and a steady stream held
the gesture open indefinitely. MIDI waits while the user's gesture is open, as
host writes do (P12).

A mapping drives a `TargetId`. A parameter is host-projected, so it takes the
audio tier. Any other state is a `Target` the plugin registers with
`Learn::register` under a key, the identity its saved mappings use, which no
parameter or other target may share. A burst is `begin`, the newest value each
poll tick that has one through `apply`, then `end`. While `Target::held` says
the user holds it, learn closes its own burst and leaves the value waiting,
checking before it takes the value, so it never takes one it cannot apply.

### L8. Mapping changes take effect at once

**Invariant.** A mapping edit, including one made from code the poll itself
runs, takes effect on the UI poll at once: a dropped target closes its burst
once and the poll never drives it again; the others still drain. The audio
thread sees the edit at its next lease, so a block already in flight can write a
target once more after the edit returns, and the snapshot must own every target
it can reach. A value staged before a mapping was cleared never lands, except
one staged by a block already in flight on the old snapshot, a one-block window
(`seam_MidiLearnEngine.h:1132-1141`); a newly mapped target starts empty; a
deferred retry applies the newest value. In C++ the retry re-raised its pending
flag with a read-modify-write, since a plain store there loses an update. Rust
has no retry (below); loom model 3 checks the staging slot instead.

**Why.** C++ held a caller-owned target by raw pointer and documented that it
must outlive the engine (`seam_MidiLearnEngine.h:159-162`, `:290-293`). Its test
of a call after detach is marked as failing (`midi_thread_test` "midi: a
caller-owned sink is never called once detach() has returned", `[!shouldfail]`),
and an engine-owned target took exactly one call after detach
(`midi_thread_test` "midi: an engine-owned sink outlives a block in flight and
goes at reclaim").

Pinned by: `midi_engine_test` "midi: a target dropped mid-poll is closed once
and never driven again", "midi: a value deferred by an undone open does not
outlive a later clear".

**Rust.** Learn calls a target only while it holds itself mutably, inside `poll`
or an edit, so a target that reaches its learn, through the session's
`Rc<RefCell<Learn>>` say, cannot edit the mappings from inside the call: the
borrow refuses. So C++'s re-entrant edits, and the retry that one of them
needed, do not exist. The table owns each UI-tier target's staging slot through
an `Arc`; every mapping, a re-point included, gets a fresh slot, so a value
staged under an earlier mapping never lands, with no one-block window. The slot
is a value and a pending flag; loom model 3 checks that a take seeing the flag
reads at least that value, so the newest value is the last one applied. The flag
is raised with a read-modify-write; a store would be correct under the memory
model, but loom 0.7.2 can lose a plain store racing a swap and report an outcome
the model forbids.

### L9. Saved mappings

**Invariant.** Mappings save as the `midiLearn` session section: schema 1, a
list sorted by target id, channel -1 meaning any, the identity curve left out. A
restore replaces every mapping in one publish, drops a bad entry on its own
without failing the rest, and restores routing, never values. An entry for an id
not registered yet is held, saved back unchanged, and applied when that id
registers, under the same checks a restore makes (`458b983`). A mapping ceiling
refuses only new mappings; re-pointing one is free and held entries take no
slot.

**Why.** A session opened before the editor or a dynamic target exists must not
lose its mappings.

**Rust.** Each entry also stores the UMP group, −1 meaning any (D21), and for an
NRPN whether its capture saw a low byte (D30), from schema 1: `{"schema": 1,
"mappings": [{"target", "kind" (cc, nrpn or note), "group", "channel", "number",
"coarse"?, "curve"?}]}`, sorted by the target's key, a parameter's key or the
key a `Target` registered with. A held entry is kept as saved, unknown fields
included. A section of another schema clears every mapping and held entry and is
then refused, so no earlier project's mapping routes MIDI and the session keeps
the section for a build that reads it (D14, D71); C++ cleared the table.
`Learn::attach` registers learn as the session's `midiLearn` participant; an
edit made inside learn's own methods reaches the session at its next tick (D44).
A restore ends any arm, since it edits every mapping (L3). Detaching a target
holds its mapping as a held entry under its key (D54); `unmap` removes one. Ids
and slots are 32-bit at least: C++ used 16-bit slot indices, which wrapped past
65,535 and made curve 65,535 collide with "no curve". Reaching that took a
mapping ceiling above 65,535, which nothing checked (default 4,096,
`seam_MidiLearnConfig.h:64`; read only at `seam_MidiLearnEngine.h:1041`).

Pinned by: `midi_persistence_test` "midi: a save before the target enrols writes
the restored entry back unchanged", "midi: a held entry takes no ceiling slot,
so an edit before it lands can take the slot".

### L10. Learn in the menu and the poll

**Invariant.** Learn contributes menu items (learn, cancel, clear, set
controller) only for registered targets, by namespaced id; an invoke resolves
the target by id and must check the target is the one the menu was built for
(C++ matched by id alone). The learn poll runs only while a target is armed, a
mapped UI-tier or relayed target exists, or a snapshot awaits reclaim, and a
steady tick allocates nothing.

**Rust.** With `triple_buffer` (D9) no snapshot awaits reclaim: a replaced table
drops at a later publish. `Learn::poll` runs whenever the plugin calls it, once
per UI tick; idle, it is one check per mapped UI-tier target and allocates
nothing, which meets the rule. The menu half waits for the menus of M1 to M3,
which arrive with the first page that needs a context menu.

Pinned by: `midi_menu_test` "The engine contributes nothing for a target it
never enrolled"; `perf_alloc_gate_test` "midi: an idle poll tick with mapped
targets is zero-alloc".

[History](decision-history.md#l10-learn-in-the-menu-and-the-poll)

### L11. A learning plugin declares MIDI input

**Invariant.** A plugin that uses learn takes MIDI input. Otherwise arming never
learns, and nothing says why.

**Why.** In C++ nothing checked: the engine never asks whether the plugin takes
MIDI, and `acceptsMidi` has followed the build's MIDI-input flag only since
`f88b359` (it was always false before). Pinned by `plugin_processor_test`
"acceptsMidi follows the target's NEEDS_MIDI_INPUT; outside juce_add_plugin it
is false" and `accepts_midi_input_test`.

**Rust.** `seam-host` will refuse a plugin that enables learn with `midi_input:
false`, which is new in Rust. The exporters do not check it yet.

[History](decision-history.md#l11-a-learning-plugin-declares-midi-input)

---

## 5. The saved block and the host boundary (seam-host)

### H1. The saved block

**Invariant.** The plugin's saved state is UTF-8 JSON with no byte-order mark or
framing: one object with `seamStateVersion` (an integer, currently 1),
`parameters` (a flat object from key to value in real units) and `extra` (the
session, left out when empty), in that order. In Rust a second version,
`pluginStateVersion`, follows the first (D84). Bool parameters write
`true`/`false`, a discrete parameter with an integral value writes an integer,
everything else a double that round-trips the f32 exactly. Only host-projected
parameters are written, in P10 order (`seam_StateJson.h:148-166`). A save may
run on any thread but the audio thread and reads only the stores and the
session's snapshots. That a choice is written as an integer is untested in C++:
the round-trip test checks the values, not the JSON types.

**Why.** Real units keep meaning across range edits; flat keys survive group
moves.

**Rust.** Byte-identical output to the C++ writer is not needed unless something
compares bytes; any reader of the same JSON gets the same values. Blocks saved
by the C++ Seam, and the JUCE wrappers around them in each format, need not load
(D5). The writer fixes the key order itself, since `serde_json`'s default map
sorts keys, and the unknown ids a load kept follow the declared parameters (D4)
and the unknown top-level keys follow `extra` (D80).
`seam_host::SavedState` holds the parameter set, whose clock its load marks
(D52), and the session, and saves and loads from any thread but audio. The
discrete parameters are bool and choice (P17), so a choice writes its index as
an integer and a stepped continuous range writes a double, 2.0 say.
`seamStateVersion` is Seam's frame, at 1, and `pluginStateVersion` the plugin's
own schema, 0 unless the plugin raises it with a migration, which upgrades a
block of an older plugin version before it loads (D80, D84). `serde_json`'s
`float_roundtrip` reads every double back to the bits it was written from, so
save, load, save writes the same bytes for any double in a section or a kept
key; without it about one value in ten drifted
by a bit on every load.

[History](decision-history.md#h1-the-saved-block)

### H2. Loading tolerates anything

**Invariant.** A block that is not recognised (empty, binary, longer than 64 MiB
or one that would save back longer, not JSON, not an object, followed by any
byte but JSON whitespace and NUL padding, a version that is missing, not a whole
number or below 1, in Rust a plugin version that is there but not a whole number
from 0, `parameters` not an object) writes nothing and raises nothing in the
plugin, and the host is told the load failed where its format can hear it (D66,
D72). A recognised block assigns every parameter: a missing value, a mistyped
one (a string, array, object or null), a non-finite one, or one outside f32's
range takes that parameter's default; anything else goes through the restore
write (P8), which snaps and clamps it to the parameter's range. A number past
f64's range, anywhere in the block, reads as null, so only that value is refused
and the rest loads. A whole-number version written as a float (1.0) loads, and
one past u64's range reads as the largest. A newer version of either loads
best-effort, never refused; an older plugin version is first upgraded by the
plugin's migration, when it has one (D80, D84). The same block always produces
the same state, learn's routing included (D71), whatever the instance held
before.

**Why.** Reproducible preset and session recall. This deliberately differs from
keeping the current value for a missing key: a load must not depend on prior
state. C++ accepted a bool, an integer or a double for a parameter of any kind
(`seam_StateJson.h:117-135`), and read a block with a byte-order mark, in
UTF-16, or with bytes after the closing brace (`juce_String.cpp:1997-2012`,
`juce_JSON.cpp:82-93`).

**Rust.** D4: keep unknown keys and save them back. A load replaces the kept
set, so the state never depends on an earlier load. JSON whitespace around the
object is part of the JSON text, which `serde_json` reads, so it loads; only NUL
padding is trimmed apart from it.

Pinned by: `state_persistence_test` "a block that omits an id restores that
parameter to its default, not its previous value", "corrupt and empty blocks are
tolerated: zero writes, no crash, no assert", "a newer format version and
unknown root keys still restore every known parameter".

### H3. Restore ordering

**Invariant.** Load: recognise the block or stop; assign every parameter
silently; hand `extra` to the session (store now, apply later); surfaces catch
up on their next poll. A restored value that lands during a drag stands until
the drag writes again, and the drag's close writes nothing (D11). In C++, on an
undo-enrolled parameter whose drag changed the value, the close wrote the drag's
last value back over the restored one as one extra host gesture
(`seam_UndoRecorder.h:834`, `:172`); unenrolled, the restored value stood.

**Rust.** The load bumps the `RestoreClock`'s stored count before its first
parameter write and again with each one (D39), so an undo between the parameter
writes and the session's store replays nothing from before the load (D20). The
session's store bumps it again; one load moving it many times is harmless, since
undo clears on any move.

Pinned by: `state_persistence_test` "a mid-gesture restore: the drag wins, and
its close may fire one host gesture (enrolled) or none (not enrolled)" (the C++
form); `plugin_processor_test` "setStateInformation applies the parameters
before any session participant".

### H4. The learn host relay

**Invariant.** Telling the host about learned writes is off by default. When on,
it brackets each burst as one host gesture (begin, the latest value per tick,
end on idle), skips a burst that starts while the user holds a gesture, stands
down when a drag has begun mid-burst (on the next tick that carries a value, or
at the idle close), and always sends begin before end.

**Why.** Most hosts do their own MIDI mapping and do not want a competing edit
stream. C++ checked for a drag only on a tick that carried a value
(`seam_MidiParameterTargets.h:151-161`, `seam_MidiLearnEngine.h:872-905`).

Pinned by: `midi_membrane_test` "midi: the host-notify relay is off by default",
"midi: the opt-in relay brackets a burst in exactly one host gesture", "midi:
the relay stays out of the way while the user owns the value".

**Rust.** The relay only notifies; it never stores again. The C++ relay
re-entered the host write from the UI thread, which could overwrite a newer
value the audio thread had stored, and sent the unsnapped value for stepped
parameters. In CLAP the relay is output events from `process`.

### H5. Smaller boundary rules

**Invariant.** The bypass parameter is the one the host drives as bypass. A
plugin declares a single program and program calls do nothing. Whether it takes
MIDI input comes from one declaration. A parameter's host name and unit come
from the same declaration the page uses. A parameter's host index is its
declaration-order position (P10), and its host id a hash of its key (D31), both
available before any editor exists.

**Not determined here.** How JUCE wraps the block in each format's state
container. It does not matter, since Rust does not load C++ blocks (D5).

---

## 6. The page (seam-bridge, page)

### B1. The page seeds after subscribing

**Invariant.** The page installs its listener, then announces itself with
`ready`, again every second until it is answered (D79); the native side answers
with every parameter's state. Changes after that arrive one by one. A page built
against another protocol version stops: it drops writes and rejects calls rather
than misreading messages. The native side answers a `ready` of a version it
cannot serve with a `reject` carrying its own version, which is what stops the
page, and a `ready` decodes whatever a newer page adds to it, so the reject
always reaches it (D78). No native side exists yet (D68); the page's half is
built.

**Why.** P14, at the page boundary. The single seed of every parameter and the
version stop are new in Rust. In C++ each page control subscribed to its own
parameter's event, then asked for that parameter's state
(`seam-web-client.js:94-115`); `page.ready` was only a command an example
registered to hide its splash (`seam_WebUIHost.h:436-447`); and no message
carried a protocol version.

Pinned by (the C++ form): `useSeamParam.test.ts` "requests a fresh reflect on
mount"; `web_parameter_attachment_test` "web bridge: a write racing the
page-load seed's read still reaches the page".

### B2. A drag owns the control

**Invariant.** The native side answers every `set` and `gestureEnd` with the
parameter's state, changed or not (D15). While a control's drag is open, a
reflected change never overwrites the value the user is dragging. When the drag
ends, the control shows what arrived during the drag, then the answer to its
end, which is the store. A one-shot set shows the user's value until its answer
arrives. A control unmounted mid-drag sends the end before it goes. Each control
sends its own begin and end; the native side collapses them (P11), which is new
in Rust. A one-shot set outside a drag gets its gesture on the native side.

**Why.** An echo of a value the drag is about to pass fought the user. A drag
left open by an unmount kept the host in automation-write mode for that
parameter indefinitely. Host writes and restores that landed during a C++ drag
were already held back on the native side and reached the page after the release
(`web_parameter_attachment_test` "web bridge: a host write mid web-drag is
deferred until the drag ends", "web bridge: without undo enrollment the deferred
host write lands at drag end"). What the C++ page lost was the snapped value of
its own edits, whose echo the native side suppressed
(`seam_WebParameterAttachment.h:159-177`, `seam_ReflectCoalescer.h:98-104`), and
any other change that arrived during the drag, which the page hook dropped
(`useSeamParam.ts:100-102`).

C++ bound one native attachment per id, with a single drag flag
(`seam_WebParameterAttachment.h:137-144`, `:182-187`); a second bind got the
first attachment back (`seam_WebUIHost.h:849-858`). Two page controls on one id
therefore did not collapse: the second control's begin did nothing, and the
first control's end closed the gesture for both. Neither saw the other's drag
values, since they shared the suppressed echo.

**Rust.** The bridge answers each `set` and `gestureEnd` from its watch on the
id, polled and then read (P12), never from the store directly: while another
surface's gesture is open on that id, the store may hold a host write every
surface is holding back. So the answer to an end is the store only when that end
closed the last gesture open on the id.

### B3. The page is typed from the declarations

**Invariant.** Every id a page passes (parameter, event, command) is checked
against the generated types, with the id as the hook's only type parameter, so a
typo is a compile error.

**Why.** Before v0.6 the hooks defaulted the map, so a bare call accepted any
id: TypeScript inferred a map containing whatever id was typed
(`types.test-d.ts:325` keeps that case for the raw hooks). v0.6 bound the map
once, in a factory (`ad03e49`). v0.7 still exported untyped `invoke`,
`bindParam` and `bindEvent` (`index.ts:46-57`), which Rust must not.

### B4. Decoding never guesses

**Invariant.** A message whose field has the wrong type or range is rejected,
never truncated or wrapped: 3.7 is not a valid integer, and neither is a
negative or oversized one. Absent and null are the same on both sides, and the
generated types say so.

**Why.** The C++ codec cast any double into an integer field (3.7 became 3, wide
values wrapped, out-of-range was undefined behaviour), and encoded an absent
optional as JSON `null` under a TypeScript type that said `undefined`, so a page
checking for `undefined` missed every absent value.

**Rust.** serde rejects the mistyped number; ts-rs types an `Option` as `T |
null`. A `set` whose position lies outside 0 to 1 is refused too, and the page
limits a position to that range before sending it. Tested in
`crates/bridge/src/protocol.rs`.

### B5. A `ready` starts a new page session

**Invariant.** Each `ready` starts a page session, which holds every gesture the
page opens. A new `ready` from the same WebView (a reload, in development or
after a crash) or the WebView going away (the editor closing) ends the old
session first: every gesture it left open closes, once, before the new `ready`
gets its snapshot.

**Why.** A page that reloads mid-drag never sends the drag's end. C++ closed a
page drag only at that control's next begin or end, or when the attachment was
destroyed with the editor (`seam_WebParameterAttachment.h:123-125`, `:137-143`,
`:182-185`). Nothing reacted to a reload, and the attachment survived it, so the
host stayed in that gesture until the editor closed, and the reloaded page's
one-shot sets on that parameter joined the stale drag (`:170-174`). Traced from
code; no C++ test covers a reload.

**Rust.** The session owns the page's gesture tokens (P11), so dropping it
closes them. Hot reload in development (D26) reloads the page often, so it
depends on this rule. A page sends `ready` again until it is answered (D79), and
holds every write until then, so no write comes before a repeat: each `ready`
may start a new session without the native side telling a repeat from a
reload.

### M1 to M3. Menus

**Invariant.**

- **M1.** A menu is flat, renderer-neutral data: items with namespaced string
  ids and parent-id nesting, in contribution order, invoked by id, never by
  position. One target identity is shared by every surface.
- **M2.** Contributors are asked in registration order and append items only for
  targets they recognise. Ids and prefixes are unique: a collision is an error
  (C++ asserted prefixes in debug only, `seam_MenuRegistry.h:270`, so in release
  a second contributor's items could not be invoked; it never checked item ids).
  An invoke routes by the id's prefix to one contributor, which re-checks its
  target, so a stale invoke is safe; "handled, nothing to do" differs from "not
  handled".
- **M3.** Reset to default is a user edit inside one gesture (one host begin and
  end, one undo step), offered disabled when the value is already at its
  default. MIDI learn contributes items only for enrolled targets; its actions
  change mappings, never the target's value, and are saved through the session.

**Why.** A right-click menu that works the same on the page and natively, and
that a stale click cannot misroute.

**Rust.** Menus cross the bridge as data and come back as an invoke. They arrive
with the first page that needs a context menu.

---

## 7. Properties the Rust tests check

Property tests generate the inputs; each property held in C++ as examples only.
Two are exceptions in Rust: property 7 is pinned by examples, and property 10 by
a seeded loop of threads, which `PROPTEST_CASES` does not reach.

1. Snapping is idempotent: a value snapped twice equals the value snapped once,
   for every kind.
2. One gesture: any writes inside one gesture give at most one step; it exists
   if and only if the final value differs from the value at open; undo gives the
   open value. For parameters and plain values (D29) alike.
3. Transactions: one entry per outermost transaction that recorded anything;
   undoing it restores every touched value.
4. Replay is pure: any sequence of undo, redo and jumps leaves the entry count
   unchanged, and the state after k applied steps is the same however k was
   reached.
5. Edit after undo: the new step's before-value is the value after the undo.
6. Host exclusion: with random host writes, program writes and gestures
   interleaved, the step count is the number of gestures with a net user change,
   and no host-written value is ever an after-value.
7. No-op writes keep redo, NaN payloads included.
8. Host gesture balance: any interleaving of begins and ends on several controls
   reaches the host as balanced, non-overlapping begin and end pairs per
   parameter.
9. Two surfaces, one dropped: the survivor follows every later host write (P15).
10. Session: random restores, registrations and releases racing saves never lose
    a section, never save an older section after a newer one, and the last
    restore wins.

---

## 8. What does not carry over

| C++ mechanism | Why Rust drops it |
| --- | --- |
| Synchronous signals: membership snapshots, a lock held across delivery, destroy-during-delivery rules, re-entrant writes from a listener | Nothing is announced synchronously; surfaces poll. |
| The reconcile pump, its timer, adaptive rate and NaN-seeded shared tracker; "inert until bound" | A change counter per parameter and a last-seen counter per surface. |
| The thread-hop marshal and its staging | The UI reads on its own thread when it polls. |
| The per-value replay flag and the replay target queue | Writes carry `Origin::Replay`. |
| The undo cascade guard and its deferred-drain window | Derivations inside a replay write with `Origin::Replay`. |
| The restore window as a broadcast signal, and its listener-order side effects | `Origin::Restore` and a shared `RestoreClock` (D19). |
| Passive whole-collection snapshots for containers that could not change type | Lists are the only collection kind. |
| Name collisions from fusing two class hierarchies (step count, text parsing) | Distinct names, and 0..1 conversions only in exporters. |
| "Register last in the most-derived constructor"; handle members declared last | A participant is fully built before it registers; `Drop` order is explicit. |
| Keeping refused objects alive so returned references stay valid | Errors, and owned values. |
| Exception swallowing in gesture close and listener calls | `Drop` must not panic; exporters catch panics at the boundary. |
| A JUCE value adapter (two-way binding to a foreign value type) | No foreign value type to adapt. |

---

## 9. Bugs found in the C++ Seam

None of these were fixed in `seam-cpp`; each is a rule above, so the Rust design
cannot reproduce it. "Traced" means derived from reading the code; nothing was
run for this transfer.

| Where | Bug | Status | Rule |
| --- | --- | --- | --- |
| Reconcile pump | No per-target registration count: two views on one parameter, one destroyed, the survivor stops following host writes | confirmed in code, untested | P15 |
| Snapping | Snap then clamp: when the end sits less than half an interval past the last lattice point, writing the end, a host 1.0 or a reload lands on that lattice point | confirmed by arithmetic; no test writes the end | P2 |
| UI bindings | Views keep the announced value, not the stored one | traced, untested | P16 |
| Learn dispatch table | Publish and lease can reorder; the reclaimer can free a snapshot the audio thread is reading | confirmed by the memory model | L4 |
| Learn | RPN data entry read as an NRPN; high-byte-only NRPN tops out at 0.9922; parser skipped while idle | confirmed | L2 |
| Learn | Arm read once per block; edits do not cancel an arm; a blocked capture hides a legal one | confirmed | L3 |
| Learn | Curves never validated (a skew of 0 flattens the curve, a negative one exceeds 1); 16-bit slot indices wrap once the unchecked mapping ceiling passes 65,535; menu resolves by id alone; MIDI writes through a user's drag on a UI-tier target | confirmed | L6, L7, L9, L10 |
| Learn | A caller-owned target is held by raw pointer: a block in flight can call it after `detach` returns | confirmed; a `[!shouldfail]` test pins it | L8 |
| Undo | Undo during a replay moves the cursor twice; redo during a replay moves it +1 then −1, so the undone entry lists as applied and the redone one as undone | traced, untested | U12 |
| Undo | A write cascaded to another value from a deferred replay drain records a new step and wipes the redo | confirmed; a test pins it as the C++ behaviour | U10 |
| Undo | A list verb from the list's own listener during a restore is a use-after-free | traced, untested | U17 |
| Undo | Watching restores from inside an open restore window records the rest of it | traced, untested | D7 |
| Undo | Double enrollment of a scalar records two steps per edit in release | confirmed | U1 |
| Undo | Transaction depth per recorder, grouping per shared history | confirmed | U6 |
| Undo | A list clear's undo has no staleness guard | confirmed | U17 |
| Session | A restore stored and applied during a capture is overwritten by that capture (the capture must pump the message loop or call `restore` on the message thread) | traced, untested | S8 |
| Learn relay | Re-stores from the UI thread and can overwrite a newer value; sends unsnapped values | confirmed | H4 |
| Parameters | Gesture begin and end checked against host attachment separately | theoretical | P11 |
| Parameters | During a drag, a user write equal to a host write not yet shown is dropped from undo | confirmed | P5 |
| Parameters | A hosted parameter reports the host's step sentinel through its base type | latent | P17 |
| Parameters, menus | A one-entry choice, and a menu prefix collision, only checked in debug builds; menu item ids never checked | confirmed | P2, M2 |
| Page codec | Any double cast into an integer field; absent encoded as `null` under an `undefined` type | confirmed | B4 |
| Page attachment | Two page controls on one id share one drag flag: the second control's begin is ignored and the first control's end closes the gesture for both | confirmed in code, untested | B2, P11 |
| Page attachment | A page control never sees another page control's drag values on the same id: both share the attachment whose echo is suppressed | confirmed in code, untested | B2 |
