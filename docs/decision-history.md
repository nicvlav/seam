# Decision history

How the decisions and rules of [the decision record](decisions.md)
were reached, and how they changed. The record states what holds now; this file
keeps the path there: who decided, what an earlier form did, and which review
or audit moved it. Each entry was its record section's paragraph headed
**Status and history.**, joined by the history clauses its other paragraphs
carried. Each links back to its section, and the record's summary table keeps
each decision's status. A decision or rule with no entry here has no history
beyond its section.

This file is closed. It runs to the first commit on `main`, and the audits,
reviews and stages it names left no other trace in this repository. A decision
or rule changed since then has its history in the commits that touched its
section of the record, which `git log` on the record shows.

The entries use these words:

- **The first form**: the code as first built, before a later change. "The form
  before" and "the form between" name later forms that were changed too.
- **Decided by the owner**: the owner chose or confirmed it. **The
  implementer's choice**: chosen while building it, and not yet confirmed.
- **The audits**: two audits of the repository on 2026-11-05, "the first audit"
  and "the second audit". An id such as HB-1 or ST-5 names one of their
  findings.
- **The review**: the review of the work that built the code in question. "The
  step 1 review" and "the step 4 review" closed those steps of the third stage.
- **Stage 2** built the core crates; **stage 3** builds the exporters, the JUCE
  adapter and the comparison against plain JUCE.

## Decisions

### D6. A participant that registers after a restore

[The decision](decisions.md#d6-a-participant-that-registers-after-a-restore)

Worded after the 2026-11-05 audits, which found the second participant's case
(ST-7); the code is as decided.

### D8. The history bound

[The decision](decisions.md#d8-the-history-bound)

The first form booked `size_of` alone, so 800 edits of a 64 KiB value kept 46
MiB under the default bound (the first audit's finding 5). Decided on
2026-11-03; amended after the 2026-11-05 audits; confirmed by the owner on
2026-11-07.

The byte cap that overrides the floor was a reviewer's suggestion in the second
audit.

### D9. Sequential consistency of the learn dispatch table

[The decision](decisions.md#d9-sequential-consistency-of-the-learn-dispatch-table)

Confirmed in `triple_buffer` 9.0.0's source when learn's table landed
(`seam_learn`'s snapshot module says where).

### D10. Learn over MIDI 2.0

[The decision](decisions.md#d10-learn-over-midi-20)

Stage 2 left pitch bend unlearnable.

### D12. Host step metadata for a stepped range

[The decision](decisions.md#d12-host-step-metadata-for-a-stepped-range)

The 2026-11-05 audits found that the C interface passes no step count for a
continuous range (RD-4).

### D16. Where the WebView pieces live

[The decision](decisions.md#d16-where-the-webview-pieces-live)

`seam-webview` was planned for stage 3.

### D18. Earlier in-house code

[The decision](decisions.md#d18-earlier-in-house-code)

When this was decided, the example plugin was the plan's first bar.

### D19. The restore counter and the applying mark

[The decision](decisions.md#d19-the-restore-counter-and-the-applying-mark)

The first form kept the mark on the clock, so a worker's `Value::set` during a
UI apply recorded nothing and took the restore's origin (ST-5). Decided on
2026-11-03; amended after the 2026-11-05 audits; confirmed by the owner on
2026-11-07.

Storing the applying thread's id in the clock was ST-5's suggestion.

### D22. Controllers reaching learn in VST3

[The decision](decisions.md#d22-controllers-reaching-learn-in-vst3)

Decided by the owner, the MIDI 2.0 pool included. The check in Cubase, first
planned for the exporter's MIDI work, ran early as a probe plugin on
2026-11-06 and passed. It found that Cubase asks only `IMidiMapping2` when a
plugin has it, so the grid, first listed through `IMidiMapping` alone, is
listed through both. Recorded 2026-11-12.

### D30. An NRPN data entry with only its high byte

[The decision](decisions.md#d30-an-nrpn-data-entry-with-only-its-high-byte)

Decided on 2026-11-03; amended after the 2026-11-05 audits, which found a low
byte sent first dropped (MD-5); confirmed by the owner on 2026-11-07.

### D33. Undo during a user gesture

[The decision](decisions.md#d33-undo-during-a-user-gesture)

In Rust's first form the replay shared the drag's bracket, and the drag's step
kept its before-value from before the replay.

### D34. A sync while a gesture is open

[The decision](decisions.md#d34-a-sync-while-a-gesture-is-open)

Taken as the review recommended; confirmed by the owner.

### D35. A reserved NRPN block

[The decision](decisions.md#d35-a-reserved-nrpn-block)

Taken as the review recommended; confirmed by the owner.

### D36. The order one undo tick records

[The decision](decisions.md#d36-the-order-one-undo-tick-records)

The first Rust form recorded parameters first, then values and lists in
enrollment order.

### D37. Whether the history keeps enrolled values alive

[The decision](decisions.md#d37-whether-the-history-keeps-enrolled-values-alive)

The first Rust form held strong handles, so an editor that enrolled its own
values and closed without forgetting them left an enrollment and a poll per
value for the life of the instance.

### D38. The origin of a newest write

[The decision](decisions.md#d38-the-origin-of-a-newest-write)

Keeping the origin in one atomic word with the value was added on 2026-11-06,
after a review showed the two stored apart could split; confirmed by the owner
on 2026-11-07. Reading both through one `ParamSet::read` was added the same day;
confirmed by the owner on 2026-11-07.

### D39. A load between a replay's check and its write

[The decision](decisions.md#d39-a-load-between-a-replays-check-and-its-write)

The first form held it for every step, so a list step took the list's lock
inside it while a thread loading inside `List::with` took them the other way
round, a deadlock a review found.

The form before moved the stored count only as a load began, so a step
recorded after that and before one of the load's writes kept a stamp the walk
still matched, and an undo wrote its pre-load value over the loaded one (a
probe: a load began over 5, a user's write of 8 recorded, the load wrote 2,
and an undo answered true and left 5). Since 2026-11-07 each load write moves
the count under the hold a replay step checks it under. The implementer's
choice; confirmed by the owner on 2026-11-07.

### D40. One undo history per UiParams

[The decision](decisions.md#d40-one-undo-history-per-uiparams)

Decided on 2026-11-03; amended after the 2026-11-05 audits; confirmed by the
owner on 2026-11-07.

The first form answered `None` to a second history.

### D41. A list step replaying against a waiting verb

[The decision](decisions.md#d41-a-list-step-replaying-against-a-waiting-verb)

Taken as the review recommended; confirmed by the owner. The first form treated
the step as stale only until the verb was taken; a redo stale for that moment
still moved the cursor, and a later step then took the wrong element.

### D42. Order across ticks

[The decision](decisions.md#d42-order-across-ticks)

Taken as the review recommended; confirmed by the owner. Without it, a worker's
push then select could record as select, then push a tick later, so one undo
left the selection past the end.

### D43. A derived change records nothing

[The decision](decisions.md#d43-a-derived-change-records-nothing)

Taken as the review recommended; confirmed by the owner. D38's first form
recorded a change derived from a user's or the plugin's own change, so a value
derived from a dragged parameter recorded a step per tick of the drag, and one
derived from a sync wiped the redo.

### D44. A participant that changes while its owner holds it

[The decision](decisions.md#d44-a-participant-that-changes-while-its-owner-holds-it)

Taken as the review recommended; confirmed by the owner. The first form did
nothing, so a mapping learned through `Learn::map` would have been lost from
every save.

The form before kept one flag for every held-up participant, which the outer
tick cleared before it captured them, so a tick made inside a held-up capture
ran its pass without capturing the others still held up, and a pass that ended
one of their registrations lost its change. Since 2026-11-07 a tick reads which
are held up from their entries. The implementer's choice; confirmed by the owner
on 2026-11-07.

### D45. Registrations when a SessionUi drops

[The decision](decisions.md#d45-registrations-when-a-sessionui-drops)

Taken as the review recommended; confirmed by the owner. The first form left
them orphaned: no later handle's tick reached them, so a restored section waited
forever and the key stayed taken.

### D46. A derived write and a plain value's open gesture

[The decision](decisions.md#d46-a-derived-write-and-a-plain-values-open-gesture)

Taken as the review recommended; confirmed by the owner. The first form rebased
the gesture for every derived write, so an echo lost the whole drag and a
mirrored host write became its before-value.

### D47. When a lone NRPN high byte settles as coarse

[The decision](decisions.md#d47-when-a-lone-nrpn-high-byte-settles-as-coarse)

Taken as the review recommended; confirmed by the owner. The first form settled
it at the end of the next audio block, so at buffers of 64 frames or fewer a low
byte two blocks late, from DIN or USB, came after a poll had learned a 14-bit
sender as coarse for good.

Amended on 2026-11-06, after a review's probe. The second form counted polls
alone: a poll that found the capture unchanged since the poll before settled
it. At blocks longer than a UI tick (1,024 frames at 44.1 kHz is 23 ms, a 60 Hz
tick 16.7 ms), two polls ran with no block between and learned a 14-bit sender
as coarse before the block that held its low byte. A lone controller 32 to 63
settled as itself the same way, before its high byte could arrive (D70). The
review suggested a count of blocks begun; the implementer counts both ends of
a block, since a poll can land after the next block began but before it offered
the low byte. Confirmed by the owner on 2026-11-07.

### D48. A plain Value as a learn Target

[The decision](decisions.md#d48-a-plain-value-as-a-learn-target)

The recommendation is the review's: tag each gesture on a `Value` as the
user's or learn's. The owner decided on 2026-11-10, choosing the shape of the
plugin boundary, that a learned controller move is never an undo step, for a
plain value as for a parameter. A plain value's target therefore opens no
gesture, which removes what the recommendation answered, and it was not
built. Recorded as decided on 2026-11-12.

### D49. A participant writing a parameter during apply

[The decision](decisions.md#d49-a-participant-writing-a-parameter-during-apply)

Taken as the review recommended; confirmed by the owner. Through `UiParams` the
write recorded an undo step in a late apply and reached the host as a user
gesture, so a host in Touch or Latch mode wrote automation during a restore.

### D50. Learn's audio side per activation

[The decision](decisions.md#d50-learns-audio-side-per-activation)

Taken as the review recommended; confirmed by the owner.

### D51. Whether a plugin must be Send

[The decision](decisions.md#d51-whether-a-plugin-must-be-send)

Decided by the owner; amended after the 2026-11-05 audits, and on 2026-11-06 for
the owner's rule that a `Send` plugin is never bound (D62); confirmed by the
owner on 2026-11-07.

### D52. Who makes the RestoreClock

[The decision](decisions.md#d52-who-makes-the-restoreclock)

Decided by the owner. The consolidated review wired a history on one clock and a
session and saved block on another: a load wrote a value and an undo wrote the
old one over it, breaking D20 and D39, and nothing failed.

### D53. What ticks a plugin with no editor open

[The decision](decisions.md#d53-what-ticks-a-plugin-with-no-editor-open)

Decided by the owner; amended after the 2026-11-05 audits, which found that off
Windows the VST3 exporter never ticked, so a host's load never reached a session
participant (the first audit's finding 3, HB-3); confirmed by the owner on
2026-11-07.

### D54. Detaching a learn target

[The decision](decisions.md#d54-detaching-a-learn-target)

Decided by the owner.

### D55. A participant whose apply panics

[The decision](decisions.md#d55-a-participant-whose-apply-panics)

Decided by the owner.

### D56. The hash that names a parameter

[The decision](decisions.md#d56-the-hash-that-names-a-parameter)

The implementer's choice, confirmed by the owner on 2026-11-07.

### D57. The lattice of a stepped range

[The decision](decisions.md#d57-the-lattice-of-a-stepped-range)

Found by the example EQ, which stored a 0 dB default as 8.94e-7; the first fix
still put 0 off zero in about a quarter of decimal ranges, -0.3 to 0.3 in steps
of 0.1 storing 5.55e-17. Decided by the owner on 2026-11-04.

### D58. What the plugin trait tells every host

[The decision](decisions.md#d58-what-the-plugin-trait-tells-every-host)

Taken from the step 1 review's list; confirmed by the owner.

### D59. The shape of the VST3 exporter

[The decision](decisions.md#d59-the-shape-of-the-vst3-exporter)

The implementer's choice, confirmed by the owner; amended after the 2026-11-05
audits, which found every call refused when a host creates the instance on one
thread and initialises it on another (the first audit's finding 2, HB-1), no
tick off Windows (finding 3), and the two exporters' engines drifting apart
(finding 4, HB-11); confirmed by the owner on 2026-11-07.

The owner decided on 2026-11-05 that one core lands before CLAP.

A release on another thread leaks, and drops its processor inside the panic
guard, since HB-7. The rule for a release from a host callback inside a tick was
added after the step 4 review. The first form's main thread was the thread that
creates the instance.

### D60. A panic inside a host's call

[The decision](decisions.md#d60-a-panic-inside-a-hosts-call)

The implementer's choice, confirmed by the owner; amended after the 2026-11-05
audits; confirmed by the owner on 2026-11-07. The first form counted per thread,
which nothing outside a test read (HB-5), and the shared core's first form kept
the mark on the instance, so a block that panicked while an activation replaced
its processor marked the new one, which then played silence until the next
activation (found by the real-time review of the audit fixes).

### D61. Who flushes denormals

[The decision](decisions.md#d61-who-flushes-denormals)

The step 1 review's recommendation; confirmed by the owner.

### D62. The C interface for the JUCE adapter

[The decision](decisions.md#d62-the-c-interface-for-the-juce-adapter)

The implementer's choice, amended after the step 4 review and again after the
2026-11-05 audits, which found an instance created on one thread and called on
another refused throughout (the first audit's finding 2, HB-1), and then that
the form after it, which trusted the header's promise that main-thread calls
never overlap, let JUCE's first tick run inside an activation on another thread,
two threads holding the plugin at once.

That a `Send` plugin is never bound is the owner's decision of 2026-11-06: under
the form before it, the first `idle` bound its thread for good, so the same host
pattern, a `prepareToPlay` off the message thread, was taken before the first
tick and refused after it.

That an overlapping tick skips without a fault and any other overlapping call
waits up to its bound is the owner's decision of the same day: under the form
between, every overlapping call was refused and counted, so a host's ordinary
prepareToPlay meeting the adapter's tick reported a fault, and failed when it
came second. The rest was confirmed by the owner on 2026-11-07.

Until 2026-11-06 a `destroy` waited for the loan as the other calls do, so one
that met another thread's tick took the plugin as the tick let go and freed the
instance while that thread was still returning through it.

Of the main-thread alternatives, the creating thread for every plugin was the
first form, the first main-thread call of any kind was the second audit's
suggestion, and the thread of the first `idle` was the form before the owner's
rule.

### D63. Keeping the C header true

[The decision](decisions.md#d63-keeping-the-c-header-true)

The implementer's choice, amended after the step 4 review and again after the
2026-11-05 audits; confirmed by the owner on 2026-11-07.

CI compiles the header as C and as C++ for x86_64 and AArch64 since HB-12, and
`CChar` answers the first audit's finding 15.

### D64. How the JUCE adapter builds a plugin

[The decision](decisions.md#d64-how-the-juce-adapter-builds-a-plugin)

The implementer's choice, amended after the step 4 review and again after the
2026-11-05 audits; confirmed by the owner on 2026-11-07.

The build once per architecture answers HB-4.

### D65. What a host sees through JUCE

[The decision](decisions.md#d65-what-a-host-sees-through-juce)

The implementer's choice, amended after the step 4 review and again after the
2026-11-05 audits; confirmed by the owner on 2026-11-07.

The bypass delay answers HB-13. An earlier form of the adapter's constructor
asserted that it ran on JUCE's message thread.

The program had no name until 2026-11-07, so the Steinberg validator failed the
adapter's VST3 build ("Programlist000-Program000: has no name", 39 of 40). It
has the plugin's name since, which leaves VST3's saved state as it was and
gives AU's preset name metadata the name, for display only. Since the same day
`getValueForText` hands the parser the text's UTF-8 in place; the first form
copied it into a `std::string` first, the same bytes. The adapter has cleared
each block's MIDI buffer since then too: without it, a plugin that declares
MIDI input and output sent its input back out. `juce_settings` has refused a
key whose id lands in JUCE's MIDI-controller range since then as well. The
implementer's choices; confirmed by the owner on 2026-11-07.

### D66. Bytes after the block, and a failed load

[The decision](decisions.md#d66-bytes-after-the-block-and-a-failed-load)

The first form refused any trailing byte and answered VST3 with success all the
same (HB-8). Decided by the owner on 2026-11-05.

That JSON whitespace around the object loads is the implementer's reading,
confirmed by the owner on 2026-11-07.

### D67. Checking the audio-thread locking rule

[The decision](decisions.md#d67-checking-the-audio-thread-locking-rule)

Decided by the owner on 2026-11-05.

The mark answers the first audit's finding 8. The typed audio-side handle was
the second audit's way to 9 for architecture; the first form kept the rules by
comment.

### D68. Audit items that wait for their code

[The decision](decisions.md#d68-audit-items-that-wait-for-their-code)

Decided by the owner on 2026-11-05.

### D69. Where a continuous range is checked

[The decision](decisions.md#d69-where-a-continuous-range-is-checked)

The first form had public fields checked only by the builder, so a range never
built into a set could panic in `Kind::steps` (the first audit's finding 17).
The implementer's choice; confirmed by the owner on 2026-11-07.

### D70. Learn's reading of MIDI 1.0 pairs

[The decision](decisions.md#d70-learns-reading-of-midi-10-pairs)

The implementer's choice; confirmed by the owner on 2026-11-07. Amended on
2026-11-06: the first form left 96 and 97 plain controllers, as C++ did; it let
only a 99 start an address, so an address sent 98 first, as JUCE sends every
NRPN, selected nothing and its data entry was dropped; and it paired a low byte
with a high byte captured any time earlier in the arm, so an independent knob 32
above learned the other one, while an arm that landed between a pair's halves
learned the low byte at once.

The owner decided on 2026-11-06 that 14-bit controller playback stays open, to
be built when an exporter delivers MIDI, and that capture waits a poll for a
pair's high byte; the pairing window is the implementer's, confirmed by the
owner on 2026-11-07.

The null number answers the first audit's finding 18, the low byte that waits
for its high byte MD-5, and the pair's high byte learned as a 7-bit controller
the first audit's finding 1. That controllers 32 to 63 also serve as plain
controllers, so capture does not guess, was found on 2026-11-06. A Reset All
Controllers that only deselects, keeping the bytes, was the first form.

A lone controller 32 to 63 first waited one poll, and two polls with no block
between learned it as itself before its high byte's block ran; it now waits as
a lone NRPN high byte does, under D47's amended rule.

### D71. Learn after an unreadable midiLearn section

[The decision](decisions.md#d71-learn-after-an-unreadable-midilearn-section)

The first form failed before clearing, so the previous project's mappings went
on routing MIDI (ST-6). The implementer's choice; confirmed by the owner on
2026-11-07.

Until 2026-11-07 learn closed the open bursts before it published the empty
table (D82).

### D72. How much a host may ask

[The decision](decisions.md#d72-how-much-a-host-may-ask)

The first form had none of the others: an endless stream or a block size near
2^31 allocated until the host aborted (the first audit's finding 14), and a host
that claimed 16 million queues, or millions of points it could not read, was
walked on the audio thread. The implementer's choice; confirmed by the owner on
2026-11-07.

Until 2026-11-06 the walk was bounded by queue index, the unknown queues
counting against the parameters, so such a host left the plugin's own unread;
and the bound on unknown queues was 1,024 for a while after, below the grid.
Until the same day a second queue for a parameter was read too and its value
replaced the first's, so the value a block took depended on where the host
listed the repeat. A setup past the frame limit was refused in both exporters,
although a longer block already played in pieces, until a reviewer recommended
building for the limit instead.

A reviewer suggested 256 MiB for the state.

### D73. What an activation makes of an open setup

[The decision](decisions.md#d73-what-an-activation-makes-of-an-open-setup)

The first forms differed: the VST3 exporter did nothing at an activation while
active, so a processor that panicked stayed silent, where the C interface
rebuilt (the first audit's finding 4, HB-11), and VST3 refused a setup with no
block size and then activated at 44.1 kHz whatever the host's rate, mistuning
every filter (HB-9). The implementer's choice; confirmed by the owner on
2026-11-07.

### D74. What export_vst3! refuses to build

[The decision](decisions.md#d74-what-export_vst3-refuses-to-build)

The first form accepted every layout, offered six channels as 5.1 and then
refused the host's 5.1 (the first audit's finding 13), and built for macOS and
Linux with no tick (finding 3). The implementer's choice; confirmed by the owner
on 2026-11-07.

The refusals of a plugin that declares no layout and of a crate built with
`panic = "abort"` came before the 2026-11-05 audits.

### D75. Making a caught fault visible

[The decision](decisions.md#d75-making-a-caught-fault-visible)

The first form counted per thread, which nothing outside a test read, so a
refused call in a real host looked exactly like a working mute (HB-5). The
implementer's choice; confirmed by the owner on 2026-11-07.

A wait that ran out has a kind of its own since 2026-11-06; it was counted as a
call on the wrong thread before, which named a thread the instance does not
have.

The throttled line per instance and kind was the second audit's suggestion.

### D76. A panic while edits are told to the host

[The decision](decisions.md#d76-a-panic-while-edits-are-told-to-the-host)

The first form cleared the flags before emitting, so a panic lost the
parameter's events and stranded the later ones, though its doc said they waited
(RT-3). The implementer's choice; confirmed by the owner on 2026-11-07.

### D77. How deep a session section may nest

[The decision](decisions.md#d77-how-deep-a-session-section-may-nest)

The first form saved any depth, and the load then refused the whole block, so
the instance came back at its defaults (ST-1). The implementer's choice;
confirmed by the owner on 2026-11-07.

The first form checked depth alone, so a participant's large capture made the
plugin save a block its own load refused (a reviewer's probe: a 64 MiB string
saved a block of 67,108,934 bytes, then `Err(TooLarge)`). Size is checked at
capture since 2026-11-06, against a budget the saved state sets; the
implementer's choice, confirmed by the owner on 2026-11-07.

That form never refused a restore, so a block within the limit could save past
it, and the same build then refused it whole (a reviewer's probes: a block of
67,108,864 bytes with no parameters, loaded into a set of 40, saved 67,109,343;
one of 20 MiB of `1e15` under an unknown top-level key saved 79,691,833). Its
budget was also unbounded until the saved state set it, so a capture registered
first skipped it (a probe: a capture of `MAX_STATE_BYTES` bytes before
`SavedState::new` saved 67,108,930 bytes, then `TooLarge`). Since 2026-11-07 a
load is refused when its block would save back past the limit, and the budget
starts at the limit and judges the held sections when it is lowered; the
implementer's choice, confirmed by the owner on 2026-11-07.

That judge dropped a section waiting for its apply but kept its restore, so the
apply saved it back (a reviewer's probe: a section restored before the saved
state was made, dropped, then 67,109,298 bytes after a tick). The load also
stored its sections and set its budget under two locks, so a tick between them
judged an apply's capture against the old budget, and the judge then dropped
another participant's restored section instead. Since 2026-11-07 the judge drops
the waiting restore too and takes unheld sections in key order, and a load
stores its sections and budget under one lock.

### D78. Protocol versions

[The decision](decisions.md#d78-protocol-versions)

The first form answered with a snapshot of the native side's version and no
parameters, which a newer page never got, since its `ready` failed to decode
(the first audit's finding 6), and the version moved by hand alone (finding 11).
The implementer's choice; confirmed by the owner on 2026-11-07.

### D79. The page client before the native side answers

[The decision](decisions.md#d79-the-page-client-before-the-native-side-answers)

The first form sent `ready` once, so a page whose `ready` came before the native
side listened waited forever (TL-13), and kept the old command's result, typed
as the new one's (TL-4). The implementer's choice; confirmed by the owner on
2026-11-07.

The form before kept whether the client was answered as a flag of its own, set
beside `ready` and the failure. Since 2026-11-07 it is read from those two,
with the same effect.

### D80. Upgrading an older saved block

[The decision](decisions.md#d80-upgrading-an-older-saved-block)

The implementer's choice, but for the version a save writes, which the owner
decided on 2026-11-06: the first form stamped the build's own version on every
save, so a version 2 block that version 1 loaded and saved reached version 2
again as a version 1 block, and its migration overwrote the kept key (a
reviewer's probe: a gain of 6 saved by version 2 came back 0); confirmed by the
owner on 2026-11-07.

The form the owner decided kept unknown parameter keys but dropped unknown
top-level keys, so a newer build's top-level content was lost under the version
that promised it (a reviewer's probe: `"future"` gone after one load and save).
Unknown top-level keys are kept since 2026-11-06, the implementer's choice;
confirmed by the owner on 2026-11-07.

The kept keys were not measured at load, so a block whose kept numbers grow as
they are written again saved past the load limit (a reviewer's probe: 20 MiB of
`1e15` saved 79,691,833 bytes). Since 2026-11-07 such a load is refused whole
(D77).

A version and a hook per session section was the first audit's plan.

Amended on 2026-11-07 by two of the owner's decisions: the version a migration
keys on is the plugin's own, `pluginStateVersion`, apart from Seam's frame
(D84), and a migration returns a `Result` with its reason (D85). The form before
keyed on `seamStateVersion` and answered a `bool`.

### D81. A registration made while a panic unwinds

[The decision](decisions.md#d81-a-registration-made-while-a-panic-unwinds)

The first form owed a pass whenever a pass or a late apply ended while its
thread was unwinding, so a registration made from a destructor during an
unrelated panic owed one, and the next tick cleared every undo history. Since
2026-11-07 only a panic that began inside the pass or the late apply owes it.
The implementer's choice; confirmed by the owner on 2026-11-07.

### D82. When learn runs a target's own code

[The decision](decisions.md#d82-when-learn-runs-a-targets-own-code)

The first form closed each target's burst before it cleared the table, so a
target whose `end` panicked inside a restore left the previous project's
mappings in place. The form between cleared first but closed before it
published, so after such a panic learn listed no mapping while the audio side
still routed the old table (a probe: the gain mapped to controller 7 and a
panicking target to 8, a burst open on 8, `restore(None)` caught unwinding,
then controller 7 at 127 still set the gain to 1). Since 2026-11-07 every
target's code runs after the table is published and the session told, in an
unmap too. The implementer's choice; confirmed by the owner on 2026-11-07.

### D83. A control's drag and one-shot set

[The decision](decisions.md#d83-a-controls-drag-and-one-shot-set)

The first form had two functions with the same body. They are one since
2026-11-07, the implementer's choice; confirmed by the owner on 2026-11-07.

### D84. The saved block's two versions

[The decision](decisions.md#d84-the-saved-blocks-two-versions)

Decided by the owner on 2026-11-07. The form before carried one number,
`seamStateVersion`, for Seam's frame and the plugin's schema both. A plugin
raised it through `with_migration`, which refused a version below Seam's 1, so
Seam could never reshape its own frame, and a block could not say whose number
it carried. That a save writes the plugin's version at 0 too, rather than
leaving it out, is the implementer's choice within the decision, as the owner
leaned to blocks that describe themselves. So is refusing a block whose plugin
version is there but not a whole number, with a `LoadError` of its own, rather
than reading it as 0. Confirmed by the owner on 2026-11-07.

### D85. A migration that cannot upgrade a block

[The decision](decisions.md#d85-a-migration-that-cannot-upgrade-a-block)

Decided by the owner on 2026-11-07. The form before returned `bool`, and its
false became `LoadError::MigrationFailed` with no reason. That the error is one
boxed type, not a type the closure chooses, is the implementer's choice within
the decision: a generic one could not be inferred from `Ok(())` or `?`.
Confirmed by the owner on 2026-11-07.

### D86. The AU codes

[The decision](decisions.md#d86-the-au-codes)

Decided by the owner on 2026-11-07. The form before was the pair
`([u8; 4], [u8; 4])`, subtype first, which a plugin could swap silently. One
struct as one field, rather than two fields, is the implementer's choice within
the decision. Confirmed by the owner on 2026-11-07.

### D87. The main bus in `ProcessSetup`

[The decision](decisions.md#d87-the-main-bus-in-processsetup)

Decided by the owner on 2026-11-07. The form before had `inputs` and `outputs`
fields.

### D88. A call a test host was refused

[The decision](decisions.md#d88-a-call-a-test-host-was-refused)

Decided by the owner on 2026-11-07. The form before answered `bool`, true for
`kResultOk` and the C interface's true. A `Refused` per host, rather than one
for both, is the implementer's choice within the decision, confirmed by the
owner on 2026-11-07. `save` followed the same day, answering a `Result` too.

### D89. How plugin code names a parameter

[The decision](decisions.md#d89-how-plugin-code-names-a-parameter)

Decided by the owner on 2026-11-10. A first build that day made a handle an
index into its set: a `Copy` id that knew its type, read against the set as
`audio.value(handle)` through a `Handle` trait, with a `Changes<N>` that held
its members' ids. Before it merged, the owner chose a parameter that carries
its store and is read with `get`, one type whether a host is shown it or not,
and the build was reworked to that on its branch.

### D90. How a plugin declares its parameters

[The decision](decisions.md#d90-how-a-plugin-declares-its-parameters)

Decided by the owner on 2026-11-10, when he was shown one small EQ declared
four ways and chose the fourth: parameters declared in a function that
returns them, typed declarations, and mistakes refused while compiling where
they can be. A proposal of typed constants gathered in a list and bound by
key was set aside the same day: it compiled only with an untyped choice
default and still wrote each parameter four times. Refusing a default outside
its range, writing a float's range as a range, and fixing ids when the set is
frozen were choices made while planning, told to the owner that day.

### D91. Who builds a plugin's parts

[The decision](decisions.md#d91-who-builds-a-plugins-parts)

The owner decided on 2026-11-10 that `Plugin::new` takes the parts Seam built
and `Plugin::saved` goes, and, with D90, that a plugin declares its parameters
in a function, so `Plugin::PARAMS` went too. That the plugin owns its parts and
ticks them, rather than the engine, was a choice made while planning, told to
the owner that day. Built on 2026-11-11, with three choices made while building
it that wait for the owner: the fault for a plugin that keeps its parts and
does not tick them, the line a refused declaration writes, and learn's refusal
of a plugin that declares no MIDI input.

Amended the same day, 2026-11-11: the owner agreed that Seam, not the plugin,
ticks the parts, through a link the engine holds weakly, and took the
amendments of a review of that plan, among them that the tick's return goes
and that the handle the engine and tests tick through is `Driver`, made by
`Driver::of`. `Parts::tick` and the fault for a forgotten tick went, so the
first of the three choices above no longer stands. Built the same day. The
other two still wait for the owner.

On 2026-11-11 the owner decided D95, which settles learn's refusal of a
plugin with no MIDI input as a panic, and adds one for learn turned on twice.
Built on 2026-11-12. The line a refused declaration writes still waits.

On 2026-11-11 the owner also agreed that Seam, not the plugin's processor,
runs learn's audio side, and took a review's amendments to that plan the same
day: it runs once per block before the first piece, not once per piece; the
link it comes through stays `Driver::of`, off `Parts`; and a borrow of learn
at the take panics rather than activating without learn. Built on 2026-11-12:
the engine takes it through `Driver::learn_audio` at each activation, and
`Parts::learn_audio` went. D50's question now names the activation, not its
processor.

The owner confirmed the decision on 2026-11-12 as built, the line a refused
declaration writes included.

### D92. When a saved plain value is captured

[The decision](decisions.md#d92-when-a-saved-plain-value-is-captured)

The owner decided on 2026-11-10 that a value saved through `Parts::save` is
captured at the tick, not at the write. Skipping a restore's write, so that a
load is captured once, was a choice made while planning, told to the owner
that day. Built on 2026-11-11, with one choice made while building it: a
value serde cannot write as JSON fails a debug assertion, and saves nothing in
a release build. The owner confirmed the decision on 2026-11-11, both choices
included. The same day the skip narrowed from any write a restore made to the
value's own apply's: a value another participant's apply wrote after its own
apply was never captured, so a save held the value from before the load.

Amended 2026-11-11: the owner decided, from a review of a plan to cut what a
plugin writes, that one call makes the value and saves it,
`Parts::saved_value(key, default)`, and that `Parts::save` goes. The rule for
when a value is captured did not change. Built on 2026-11-12.

### D93. A derive that writes `declare`

[The decision](decisions.md#d93-a-derive-that-writes-declare)

The owner decided on 2026-11-10 that a derive writes `declare`, with this
notation and the kind taken from the field's type, landing after the
hand-written form, and that a mistake in a declaration is a compile error
where it can be. Built on 2026-11-11, with two choices made while building it:
the derive checks option and flag names against a list a test keeps equal to
the declarations' methods, and it refuses a generic struct rather than write
code that cannot compile. The owner confirmed both on 2026-11-11.

### D94. A derive that writes `Choices`

[The decision](decisions.md#d94-a-derive-that-writes-choices)

The owner decided on 2026-11-11, from a review of the plan, that a derive
writes `Choices`, that a variant's label is its name unless a mark gives one,
and that a written discriminant must equal the variant's position. The same
review moved the label check onto the enum and refused an enum of one variant
there. Built on 2026-11-11, with two choices made while building it: a
`#[choice]` mark on the enum itself is refused, where the compiler would
otherwise accept it and ignore it; and each label is checked on itself rather
than once on the enum, so the error names the label at fault, after the owner
asked the same day to take what was worth taking from a general enum-naming
derive. The owner confirmed both on 2026-11-12.

### D95. Mistakes panic, failures return a `Result`

[The decision](decisions.md#d95-mistakes-panic-failures-return-a-result)

The owner decided on 2026-11-11, from a review of a plan to cut what a plugin
writes, that a mistake fixed in the plugin's source panics and a failure that
can depend on data returns a `Result`, and that `Parts::undo` and
`Parts::midi_learn` follow it. Built on 2026-11-12, with no choice made while
building it beyond the wording of the two panics. The owner confirmed it on
2026-11-12.

### D96. Parameters made at run time

[The decision](decisions.md#d96-parameters-made-at-run-time)

The owner decided on 2026-11-10 that a plugin makes parameters while it runs:
the same types, from the same declarations, owned by whatever holds them, with
no pool and no flag. On 2026-11-12, from a working build, the owner decided
that each is saved by its path, that `id()` stays and panics for one made at
run time, that a cell shares nothing, that two live parameters with one path
are a fault, and the run-time names. The same day, from a review of the
design, the owner decided that a load writes none of them on the host's
thread, and that `Gesture::id` returns an `Option`. A typed write for a
gesture and a `TargetId` made from a typed parameter were approved and left
until something calls them, and so were `watch` and `sync` taking one. Built
on 2026-11-12; the choices the record lists as to confirm wait for the owner.

## Rules

### P7. The host write: sanitise, clamp, convert, snap, store, publish

[The rule](decisions.md#p7-the-host-write-sanitise-clamp-convert-snap-store-publish)

Before the 2026-11-05 audits nothing failed on x86 when either ordering was
weakened (RT-1).

### P17. Host-facing metadata

[The rule](decisions.md#p17-host-facing-metadata)

The first form counted a value's places from the step's first significant digit,
so on a step of 0.125 it showed 0.125 as "0.12". A later form counted the step's
decimals alone, so a whole step from 0.5 showed 1.5 as "2", which read back as
2.5.

### U19. A bounded history

[The rule](decisions.md#u19-a-bounded-history)

The test that checks the booking against the documented numbers answers ST-8.

### L4. The dispatch snapshot

[The rule](decisions.md#l4-the-dispatch-snapshot)

The second audit found this paragraph read as if the fallback and its models
existed in the code.

### L10. Learn in the menu and the poll

[The rule](decisions.md#l10-learn-in-the-menu-and-the-poll)

The first page that needs a context menu was planned for stage 3.

### L11. A learning plugin declares MIDI input

[The rule](decisions.md#l11-a-learning-plugin-declares-midi-input)

The check was planned for stage 3, with the exporters.

### H1. The saved block

[The rule](decisions.md#h1-the-saved-block)

Writing a choice as an integer answers ST-14, and `float_roundtrip` ST-2.

Since 2026-11-07 the block carries `pluginStateVersion` after
`seamStateVersion`, the owner's decision (D84).
