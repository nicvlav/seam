# Glossary

The words Seam coins, or uses in a sense of its own. Each entry says what the
word means here and links the item or doc that owns it. An item's own docs are
the contract; this page is the map.

### Activation

The host's call that builds a processor:
[`Plugin::activate`](../crates/host/src/plugin.rs) takes a `ProcessSetup`, the
rate, block size and main bus layout the host chose, and allocates everything
the audio thread will touch. Deactivation hands the processor back to the main
thread, so what it holds is never freed on the audio thread.

### Applying mark

The mark a thread holds while it applies a restore to a session participant, so
writes made on that thread record no undo step. It belongs to the thread, not
the plugin: another thread's writes meanwhile record as usual
([`RestoreClock`](../crates/params/src/clock.rs)).

### Arm

To make a learn target take the next controller that moves:
[`Learn::arm`](../crates/learn/src/mappings.rs) names the target and what it
takes (`Takes`). A new arm ends the last one.

### Audio-thread mark

A debug-build mark an exporter raises around everything a block runs. Under it,
a call to one of Seam's locking types, a `Value` or a `Session` say, panics
([`seam_params::audio_thread`](../crates/params/src/audio_thread.rs)).

### Block

Three senses:

- **A block**: one `process` call's audio and MIDI.
  [`Block`](../crates/host/src/audio.rs) holds the channels in place, the frame
  count and the events in and out.
- **The saved block**: the plugin's saved state, the JSON object a host stores
  and hands back ([`SavedState`](../crates/host/src/state.rs)). It names two
  versions: `seamStateVersion`, of the frame Seam gives it, and
  `pluginStateVersion`, of the plugin's own schema, which a migration raises.
- **A reserved block**: a range of NRPN numbers the plugin's own protocol owns
  (see [reservation](#reservation)).

### Burst

A run of learned values for one UI-tier [target](#target), written as one
gesture, so a controller sweep is one undo step. It closes after 15 ticks with
no new value, or as soon as the user takes the control
([`Target`](../crates/learn/src/target.rs)).

### Capture

Two senses:

- **Session**: a participant's
  [`Participant::capture`](../crates/session/src/lib.rs) returns its
  [section](#section) for the next save.
- **Learn**: while a target is [armed](#arm), the audio thread captures the
  newest legal message, which the next poll commits
  ([capture.rs](../crates/learn/src/capture.rs)).

### Coarse, fine

The two final states of a learn capture. An NRPN whose sender sends no low byte
is coarse: its binding reads the high byte as 7-bit, so the sender still reaches
the top of the range (the `coarse` field of
[`Mapping`](../crates/learn/src/mappings.rs), set by `Mapping::as_coarse`).
Every other final capture is fine, an NRPN whose low byte arrived included
([capture.rs](../crates/learn/src/capture.rs)).

### Drain

The one consumer of a queue, which hands each item out once; a second is refused
while the first lives. [`HostEventDrain`](../crates/params/src/host_events.rs)
hands an exporter the begins, values and ends the host must hear;
[`UiParams::take_gesture_records`](../crates/params/src/ui.rs) hands undo the
gestures that closed.

### Edit

A program write that opens a gesture, so the host hears a begin, the values and
an end, as for a user's drag. It runs on the UI thread and records no undo step
([`UiParams::edit`](../crates/params/src/ui.rs)). In "the plugin's edits reach
the host" the word covers user writes too: every write the host must be told of.

### Enrollment

Putting a parameter, a plain value or a list under an undo history
([`History::enroll_param`, `History::enroll`](../crates/undo/src/enroll.rs)).
Enrolling records nothing; changes after it do.

### Entry, step

A step is one recorded change of one value. An entry is what one undo undoes:
one step, or every step a [transaction](#transaction) recorded
([`History`](../crates/undo/src/history.rs)).

### Exporter

The code that turns one plugin format's calls into calls on a plugin: VST3, and
the C interface the JUCE adapter drives; CLAP's is planned, not built. The
exporters are thin shells over one core, the engine
([seam-host](../crates/host/src/lib.rs)).

### Fault

A panic or a refused call an exporter caught at its boundary. It is counted for
the instance and the process, and the first of each kind is reported as one line
([faults.rs](../crates/host/src/faults.rs)).

### Gesture

A user's touch on one control, from pointer down to pointer up.
[`UiParams::begin`](../crates/params/src/ui.rs) opens a `Gesture`; gestures on
one parameter nest and count, so the host hears one begin and one end and undo
records one step. A plain [`Value`](../crates/params/src/value.rs) has gestures
of its own, with no host to tell.

### Held

Several senses:

- **Held back**: while a gesture is open on a parameter, its surfaces do not
  show host writes, restores, learned writes or syncs. They show at the first
  poll after the close ([the
  rule](decisions.md#p12-an-open-gesture-holds-back-host-shaped-writes-from-surfaces)).
- **Held by the user**: [`Target::held`](../crates/learn/src/target.rs) says the
  user holds the target's control. Learn then closes its burst and leaves the
  newest value waiting until they let go.
- **A held entry**: a learn mapping kept under its target's key while no target
  with that key is registered, after a restore or a
  [`Learn::detach`](../crates/learn/src/mappings.rs). It is saved back unchanged
  and applied when such a target registers
  ([mappings/session.rs](../crates/learn/src/mappings/session.rs)).
- **Held address bytes**: the NRPN address bytes learn's parser keeps, per group
  and channel, until an address change or a reset forgets them ([the
  decision](decisions.md#d70-learns-reading-of-midi-10-pairs)).
- **Held by the page**: a page holds its writes and calls until the native
  side's snapshot arrives, then sends them in the order it made them
  ([held.ts](../page/src/held.ts)).
- **Held up**, and a **faulted** participant: a session participant is held up
  when its own code had it borrowed as a capture came, so the next tick captures
  it. One whose apply panicked is faulted: the session ignores its captures
  until an apply succeeds. Both are an entry's `Status`
  ([ui.rs](../crates/session/src/ui.rs)).
- **Held**, a decision's status: the work does not need the decision yet, so its
  section keeps a recommendation ([the decision
  record](decisions.md#overview)).

### Learned write

A write from MIDI learn, with [`Origin::Learn`](../crates/params/src/origin.rs).
It lands on the audio thread in the block its message arrived in, opens no
gesture, records no undo step and tells the host nothing
([`LearnAudio`](../crates/learn/src/audio.rs)).

### Loan

How the C interface shares a plugin that is `Send` between threads. Each
main-thread call takes the plugin on loan for its length, on whichever thread it
arrives. A call that meets another's loan waits up to 50 ms, and a tick skips
([`Engine`'s threading rule](../crates/host/src/engine/mod.rs); the state that
is lent, and the `Patience` that sets the wait, are `MainThread` in
[boundary.rs](../crates/host/src/boundary.rs); `SeamPlugin` in
[seam.h](../crates/host/include/seam.h) for C users).

### Main thread

The thread a plugin is built, ticked and dropped on. In VST3 it is the thread
that calls `initialize`. Through the C interface it is the creating thread for a
plugin that is not `Send`, and any thread, on [loan](#loan), for one that is
([Threads](../ARCHITECTURE.md#threads)).

### Origin

Who wrote a value: the host, a restore, a learned controller, a user, the
plugin's own code, or an undo replay. Every write names one, and the origin
alone decides the write's side effects
([`Origin`](../crates/params/src/origin.rs)).

### Participant

An owner of saved state beyond parameters, such as learn's mappings or an
editor's layout. It saves one [section](#section) under a stable key, and a
restore applies to it on a later tick
([`Participant`](../crates/session/src/lib.rs)).

### Pass

A session's apply pass. On a tick,
[`SessionUi::tick`](../crates/session/src/ui.rs) applies the newest stored
restore to every participant in registration order. Restores stored before the
tick collapse into one pass, and passes never nest.

### Piece

Part of a block. A block longer than the activation's `max_frames` reaches the
processor as several calls of at most that many frames
([`ProcessSetup`](../crates/host/src/audio.rs)).

### Reading

One look at a value: its change count, the value and the origin of the write
that stored it, taken together so none is newer than another
([`Reading`](../crates/params/src/origin.rs), from `ParamSet::read` and
`Value::read`).

### Replay

Undo or redo applying a step again, with
[`Origin::Replay`](../crates/params/src/origin.rs). The host hears it as one
complete gesture, and it never records a step of its own.

### Reservation

A range of controllers or NRPN numbers that the plugin's own protocol owns.
Learn never captures a
[`CcReservation`](../crates/learn/src/reservation.rs), and never captures or
dispatches an [`NrpnReservation`](../crates/learn/src/reservation.rs); both go
in learn's `Config`.

### Restore

Loading a saved state, with [`Origin::Restore`](../crates/params/src/origin.rs).
Parameters are written at once and silently. The session stores its sections at
once and applies them on a later tick
([`Session::restore`](../crates/session/src/session.rs)), and undo clears its
history.

### Restore clock

The plugin's count of restores stored and completed, which undo reads to clear
itself, and the one sequence that orders what undo records. The parameter set
makes it, so every part of one plugin shares one
([`RestoreClock`](../crates/params/src/clock.rs)).

### Room

The fixed capacity a block's automation has: 8 points per parameter on average,
shared by the block's queues and fixed when the engine is built. A parameter's
first queue wins and its last point lands; a point past the room is counted as
dropped ([automation.rs](../crates/host/src/engine/automation.rs)).

### Section

One participant's part of the saved block's `extra` object, a JSON value under
its key ([`Section`](../crates/session/src/lib.rs)). A section no participant
claims is kept and saved back, and applied to the next participant that
registers its key.

### Snapshot

Two senses: learn's dispatch snapshot, the table of mappings the audio thread
reads ([snapshot.rs](../crates/learn/src/snapshot.rs)), and the page protocol's
`snapshot` message: the native side's answer to a page's `ready`, with every
parameter's state ([seam-bridge](../crates/bridge/src/lib.rs)).

### Staged

Waiting on the audio thread for the UI poll: a message captured while a target
is armed, or the newest learned value for a UI-tier target. The poll commits or
writes it ([capture.rs](../crates/learn/src/capture.rs)); the bit layout of a
staged word is [field.rs](../crates/learn/src/field.rs).

### Store

A parameter's one value cell, in real units. The audio thread, the UI, the host
and the save path all read it, so none sees a value older than another. The
value and its origin share one atomic word. Its set and its [typed
parameter](#typed-parameter) share it
([`ParamSet`](../crates/params/src/set.rs);
[param_cell.rs](../crates/params/src/set/param_cell.rs),
[value_cell.rs](../crates/params/src/set/value_cell.rs)).

### Surface

One presentation of a value: a control on the page, or a native widget. Each
keeps its own place in the parameter's change count, so dropping one never
starves another ([watch](#watch)).

### Sync

A program write that opens no gesture and asks the host to re-read values, for a
preset recall or a randomise. It runs on any thread but audio
([`ProgramParams::sync`](../crates/params/src/program.rs)).

### Target

What a learn mapping drives: a parameter, written on the audio thread, or a
target the plugin registers, written on the UI thread in [bursts](#burst)
([`Target`, `TargetId`](../crates/learn/src/target.rs)).

### Tick

One call of [`Plugin::idle`](../crates/host/src/plugin.rs), about 60 times a
second on the main thread for the instance's life, editor open or not. "Once per
UI tick" means from that call: `Learn::poll`, `SessionUi::tick` and
`History::tick` run there.

### Tier

Two senses. The crates' tiers are the core, the projections and the shells,
dependencies pointing down only
([ARCHITECTURE.md](../ARCHITECTURE.md#architecture)). A learn target's tier says
which thread writes it: a parameter's learned value lands on the audio thread, a
registered target's on the UI thread.

### Transaction

Several steps undone and redone as one entry: every step recorded while a
transaction is open joins it
([`History::transaction`](../crates/undo/src/history.rs)).

### Typed parameter

A parameter as plugin code holds it: a [`Float`, `Bool` or
`Choice`](../crates/params/src/param.rs) that carries its [store](#store) and
reads, with `get`, as the type its kind stands for: an `f32`, a `bool` or the
plugin's enum. A clone is the same parameter. Its
[`ParamId`](../crates/params/src/decl.rs) is its place in its set.

### Watch

One surface's view of one parameter, polled on each tick. It shows the store, or
while a gesture is open the gesture's own value
([`Watch`](../crates/params/src/watch.rs)).
