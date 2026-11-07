# Decision index

This index maps each decision and rule of the decision record
([transfer-from-seam-juce.md](transfer-from-seam-juce.md)) to the code and tests
behind it. Look an id up here to go from a decision to its code; the record says
why, and [decision-history.md](decision-history.md) how it was reached. Code and
its comments carry no ids. Each heading is the title of the matching section of
the record, and each entry opens with a link to that section.

Each line names a file and the items in it (a function, type, method, field or
test, by name; "module" for the file's own doc), and stays one line, since
`scripts/check-decision-index.py` reads it whole. The script fails CI when a
listed file or item no longer exists, or when an id in the record is missing
here. An id with nothing built for it yet says "no code site". When code moves,
move its line here; when a decision gains code, add one.

## Decisions

### D1. A plugin's own write and the host

[Record section](transfer-from-seam-juce.md#d1-a-plugins-own-write-and-the-host)

- `crates/host/src/cabi/abi.rs`: `RESTART_VALUES`
- `crates/host/src/engine/mod.rs`: `Engine::tell_host`
- `crates/params/src/origin.rs`: `Origin`
- `crates/params/src/program.rs`: `ProgramParams`
- `crates/params/src/ui.rs`: module, `UiParams::edit`, `Edit`
- test `crates/host/src/vst3/tests.rs`: `a_sync_made_while_no_handler_is_linked_is_told_to_the_next_one`
- test `crates/params/tests/writes.rs`: module, `a_user_write_inside_a_program_edit_starts_from_the_edits_value`, `a_sync_from_another_thread_asks_for_a_rescan_and_opens_nothing`

### D2. Undo's before-state on mid-gesture enrollment

[Record section](transfer-from-seam-juce.md#d2-undos-before-state-on-mid-gesture-enrollment)

- `crates/params/src/events.rs`: `GestureRecord::before`
- `crates/params/src/ui.rs`: module, `ParamGesture::user_before`
- `crates/params/src/value.rs`: `Commit`, `Value`
- `crates/undo/src/enroll.rs`: module, `History::enroll_param`, `History::enroll`
- test `crates/params/tests/writes.rs`: module
- test `crates/undo/tests/enroll.rs`: module, `a_gesture_open_at_enrollment_records_from_its_open_value`, `a_plain_value_gesture_records_from_its_first_open_after_what_came_before`

### D3. Undo during a replay

[Record section](transfer-from-seam-juce.md#d3-undo-during-a-replay)

- `crates/undo/src/history.rs`: module, `History`
- test `crates/undo/src/history/tests.rs`: `a_request_made_during_a_replay_is_refused_and_the_cursor_moves_once`

### D4. Unknown parameter ids in a loaded block

[Record section](transfer-from-seam-juce.md#d4-unknown-parameter-ids-in-a-loaded-block)

- `crates/host/src/state.rs`: module, `SavedState::last`, `LastLoad::unknown`, `SavedState::save`, `SavedState::load`
- test `crates/host/tests/state.rs`: module, `unknown_keys_are_kept_saved_after_the_declared_and_replaced_by_the_next_load`, `a_load_with_no_unknown_keys_empties_the_kept_set`

### D5. Reading blocks saved by the C++ Seam

[Record section](transfer-from-seam-juce.md#d5-reading-blocks-saved-by-the-c-seam)

- no code site

### D6. A participant that registers after a restore

[Record section](transfer-from-seam-juce.md#d6-a-participant-that-registers-after-a-restore)

- `crates/session/src/session.rs`: `State::answered`, `Session::restore`
- `crates/session/src/ui.rs`: `SessionUi::register`
- test `crates/session/tests/session.rs`: `a_late_participant_the_newest_restore_lacked_is_reset_once`

### D7. Restore neutrality for undo

[Record section](transfer-from-seam-juce.md#d7-restore-neutrality-for-undo)

- `crates/session/src/lib.rs`: module
- `crates/undo/src/history.rs`: `History::observe_clock`
- `crates/undo/src/lib.rs`: module
- test `crates/undo/tests/restore.rs`: module

### D8. The history bound

[Record section](transfer-from-seam-juce.md#d8-the-history-bound)

- `crates/undo/src/enroll.rs`: `ParamStep`, `ValueStep`
- `crates/undo/src/eq.rs`: `undo_by_eq`, `undo_by_fields`
- `crates/undo/src/history.rs`: module, `STEP_OVERHEAD`, `booked`
- `crates/undo/src/lib.rs`: `Bound`
- `crates/undo/src/list.rs`: `InsertStep`, `RemoveStep`, `ClearStep`
- `crates/undo/src/size.rs`: module, `UndoSize`
- test `crates/undo/src/history/tests.rs`: `a_step_books_its_values_plus_32_once_and_pruning_drops_whole_entries`, `a_scope_whose_entry_is_pruned_away_starts_a_new_one`
- test `crates/undo/tests/bound.rs`: module, `eight_hundred_parameter_drags_keep_what_the_default_bound_implies`, `eight_hundred_edits_of_a_small_value_keep_what_the_default_bound_implies`, `eight_hundred_edits_of_a_64_kib_value_stay_under_the_bound`, `eight_hundred_removals_of_64_kib_elements_stay_under_the_bound`, `a_clear_books_every_element_it_took`
- test `crates/undo/tests/list.rs`: `a_clear_books_every_element_it_holds`

### D9. Sequential consistency of the learn dispatch table

[Record section](transfer-from-seam-juce.md#d9-sequential-consistency-of-the-learn-dispatch-table)

- `Cargo.toml`: `[workspace.dependencies]`
- `crates/learn/src/lib.rs`: module
- `crates/learn/src/snapshot.rs`: module

### D10. Learn over MIDI 2.0

[Record section](transfer-from-seam-juce.md#d10-learn-over-midi-20)

- `crates/learn/src/lib.rs`: module
- `crates/learn/src/parse.rs`: module
- `crates/learn/src/scale.rs`: module
- test `crates/learn/src/scale.rs`: `a_midi_1_controller_reads_the_same_in_a_hosts_midi_2_translation`, `linear_midi_2_values_stay_within_a_bound_of_centre_anchoring`
- test `crates/learn/tests/dispatch.rs`: module, `seven`, `midi_2_values_read_linearly_and_assignable_controllers_are_nrpns`

### D11. A drag's close and a restore that landed during it

[Record section](transfer-from-seam-juce.md#d11-a-drags-close-and-a-restore-that-landed-during-it)

- `crates/params/src/ui.rs`: `UiState::close`
- `crates/params/src/value.rs`: `Value::close`
- `crates/undo/src/enroll.rs`: module
- test `crates/params/tests/writes.rs`: module, `the_close_writes_nothing_so_a_host_write_during_the_drag_stands`
- test `crates/undo/tests/enroll.rs`: module, `a_gesture_is_one_step_and_a_host_write_during_it_never_joins`
- test `crates/undo/tests/restore.rs`: `a_parameter_drag_across_a_restore_keeps_its_before_and_the_restore_stands`

### D12. Host step metadata for a stepped range

[Record section](transfer-from-seam-juce.md#d12-host-step-metadata-for-a-stepped-range)

- `adapters/juce/SeamPluginProcessor.cpp`: `PluginProcessor`
- `crates/host/src/cabi/abi.rs`: `KIND_CONTINUOUS`
- `crates/host/src/cabi/mod.rs`: `param_info`
- `crates/host/src/vst3/component/controller.rs`: `Component::getParameterInfo`
- `crates/host/src/vst3/mod.rs`: module
- `crates/params/src/meta.rs`: module, `Kind::steps`, `Kind::is_discrete`
- test `crates/params/tests/meta.rs`: module, `only_bool_and_choice_are_discrete`

### D13. When a session restore applies

[Record section](transfer-from-seam-juce.md#d13-when-a-session-restore-applies)

- `crates/session/src/lib.rs`: module, `Participant::apply`
- `crates/session/src/session.rs`: `Session::restore`
- `crates/undo/src/lib.rs`: module
- test `crates/session/tests/session.rs`: module, `a_restore_stores_every_section_at_once_and_applies_on_a_later_tick`
- test `crates/undo/tests/restore.rs`: module, `an_undo_between_a_loads_writes_and_its_store_replays_nothing`, `steps_recorded_between_a_store_and_its_apply_are_dropped_at_completion`

### D14. A participant whose apply fails

[Record section](transfer-from-seam-juce.md#d14-a-participant-whose-apply-fails)

- `crates/learn/src/mappings/session.rs`: `Learn::restore`
- `crates/session/src/lib.rs`: module, `Participant::apply`
- `crates/session/src/ui.rs`: `UiState::apply`
- test `crates/learn/tests/saved.rs`: `a_section_of_another_schema_is_refused_and_clears_the_routing`
- test `crates/session/tests/session.rs`: module, `a_failed_or_panicking_apply_keeps_its_state_and_raw_section_and_the_others_apply`, `a_failed_apply_does_not_hold_its_participant`, `a_failed_late_apply_keeps_the_raw_section_and_the_registration_stands`

### D15. The page's state after its own write

[Record section](transfer-from-seam-juce.md#d15-the-pages-state-after-its-own-write)

- `crates/bridge/src/lib.rs`: module

### D16. Where the WebView pieces live

[Record section](transfer-from-seam-juce.md#d16-where-the-webview-pieces-live)

- no code site

### D17. Learning notes

[Record section](transfer-from-seam-juce.md#d17-learning-notes)

- `crates/learn/src/capture.rs`: `Takes`, `NOTE`
- `crates/learn/src/lib.rs`: module
- `crates/learn/src/mappings.rs`: `Learn::arm`
- `crates/learn/src/parse.rs`: module, `Parser::parse`
- `crates/learn/src/source.rs`: `Control::Note`
- test `crates/learn/tests/capture.rs`: `a_note_arm_learns_a_note_on_and_never_a_note_off`
- test `crates/learn/tests/dispatch.rs`: module, `a_note_on_writes_its_velocity_and_a_note_off_writes_nothing`

### D18. Earlier in-house code

[Record section](transfer-from-seam-juce.md#d18-earlier-in-house-code)

- no code site

### D19. The restore counter and the applying mark

[Record section](transfer-from-seam-juce.md#d19-the-restore-counter-and-the-applying-mark)

- `crates/params/src/clock.rs`: module, `MARKS`, `RestoreWriter::applying`, `ApplyingGuard`, `RestoreReader::applying`
- `crates/params/src/lib.rs`: module
- `crates/session/src/lib.rs`: module
- `crates/session/src/session.rs`: `Session::new`, `Session::restore`
- `crates/session/src/ui.rs`: `SessionUi::tick`
- `crates/undo/src/lib.rs`: module
- test `crates/params/tests/clock.rs`: module, `the_applying_mark_belongs_to_the_thread_that_took_it`, `a_mark_on_one_clock_is_not_a_mark_on_another`
- test `crates/session/tests/session.rs`: module, `the_session_writes_the_restore_clock`
- test `crates/undo/tests/restore.rs`: module

### D20. The window between a restore's store and its apply

[Record section](transfer-from-seam-juce.md#d20-the-window-between-a-restores-store-and-its-apply)

- `crates/host/src/state.rs`: `SavedState::load`
- `crates/params/src/clock.rs`: module, `RestoreReader::stamp`, `Stamped`
- `crates/params/src/events.rs`: `GestureRecord::stamp`
- `crates/params/src/program.rs`: `ProgramParams::load`, `Load`
- `crates/params/src/ui.rs`: `GestureRecords::take`
- `crates/session/src/lib.rs`: module
- `crates/session/src/session.rs`: `Session::restore`
- `crates/session/src/ui.rs`: `SessionUi::tick`
- `crates/undo/src/enroll.rs`: `Enrolled::poll`, `History::tick`
- `crates/undo/src/history.rs`: `State::seen`, `History::entries`, `History::undo`, `History::observe_clock`
- `crates/undo/src/lib.rs`: module
- test `crates/params/tests/clock.rs`: module
- test `crates/params/tests/writes.rs`: `a_step_carries_the_restore_stamp_at_its_close`
- test `crates/session/tests/session.rs`: `a_pass_that_left_a_busy_participant_owed_is_not_completed`
- test `crates/undo/src/history/tests.rs`: `a_replay_stops_before_its_next_write_when_a_restore_is_stored`
- test `crates/undo/tests/restore.rs`: module, `history_from_before_a_restore_is_cleared_and_never_replays`, `steps_recorded_between_a_store_and_its_apply_are_dropped_at_completion`, `a_plain_value_gesture_across_a_restore_moves_the_baseline_with_no_write_after`, `list_verbs_made_after_a_reset_an_apply_or_a_restore_undo_as_usual`, `plain_value_writes_coalesce_only_within_one_stamp`, `the_listing_counter_reads_the_clock`

### D21. What a learn binding records of its MIDI source

[Record section](transfer-from-seam-juce.md#d21-what-a-learn-binding-records-of-its-midi-source)

- `crates/learn/src/lib.rs`: module
- `crates/learn/src/saved.rs`: module
- `crates/learn/src/source.rs`: module, `Source`
- `crates/learn/src/table.rs`: module
- test `crates/learn/src/table.rs`: `the_lookup_finds_what_a_scan_finds`
- test `crates/learn/tests/dispatch.rs`: module, `groups_and_channels_stay_apart_and_any_matches_every_one`
- test `crates/learn/tests/saved.rs`: module, `the_section_is_schema_1_sorted_by_key_with_the_identity_curve_left_out`

### D22. Controllers reaching learn in VST3

[Record section](transfer-from-seam-juce.md#d22-controllers-reaching-learn-in-vst3)

- `crates/params/src/host_id.rs`: `RESERVED_HOST_IDS`
- `crates/params/src/lib.rs`: module
- `crates/params/src/set.rs`: `DeclError::ReservedHostId`
- test `crates/params/tests/set.rs`: `a_key_whose_host_id_is_reserved_or_taken_is_refused`, `the_reserved_range_is_pinned_at_both_edges`

### D23. How the processor sends MIDI

[Record section](transfer-from-seam-juce.md#d23-how-the-processor-sends-midi)

- `crates/host/src/audio.rs`: `Block`
- `crates/host/src/engine/process.rs`: `pieces`, `share`
- `crates/host/src/lib.rs`: module
- `crates/ump/src/lib.rs`: `EventList::as_mut_slice`, `EventList::add_dropped`
- test `crates/host/src/engine/process/tests.rs`: `each_piece_gets_its_own_events_and_what_it_sends_lands_at_the_block_s_frames`, `a_block_of_one_piece_hands_its_events_on_and_the_next_starts_clean`

### D24. Answering MIDI-CI

[Record section](transfer-from-seam-juce.md#d24-answering-midi-ci)

- test `crates/learn/tests/dispatch.rs`: `every_packet_reaches_the_processor_untouched_and_in_order`

### D25. The SysEx manufacturer ID

[Record section](transfer-from-seam-juce.md#d25-the-sysex-manufacturer-id)

- no code site

### D26. Loading the page from a dev server

[Record section](transfer-from-seam-juce.md#d26-loading-the-page-from-a-dev-server)

- no code site

### D27. What runs the generator

[Record section](transfer-from-seam-juce.md#d27-what-runs-the-generator)

- `xtask/src/lib.rs`: module

### D28. Threads that may make a program write

[Record section](transfer-from-seam-juce.md#d28-threads-that-may-make-a-program-write)

- `crates/params/src/host_events.rs`: `HostQueue::request_rescan`
- `crates/params/src/origin.rs`: `Origin`
- `crates/params/src/program.rs`: `ProgramParams`
- `crates/params/src/set.rs`: `ParamSet::take_rescan`
- `crates/params/src/ui.rs`: module, `Edit`
- test `crates/params/src/set/tests.rs`: `loom_a_user_write_and_a_sync_publish_their_count_and_request_after_their_value`
- test `crates/params/tests/watch.rs`: `an_open_program_edit_holds_back_a_host_write`
- test `crates/params/tests/writes.rs`: module, `a_program_edit_during_a_drag_shares_the_drags_bracket`, `a_sync_from_another_thread_asks_for_a_rescan_and_opens_nothing`

### D29. Gestures on plain values

[Record section](transfer-from-seam-juce.md#d29-gestures-on-plain-values)

- `crates/params/src/lib.rs`: module
- `crates/params/src/value.rs`: module, `Value`
- `crates/undo/src/enroll.rs`: module, `History::enroll`
- test `crates/undo/tests/enroll.rs`: module, `undo_waits_while_a_plain_value_gesture_is_open`, `a_plain_value_gesture_records_from_its_first_open_after_what_came_before`, `a_plain_value_gesture_back_to_its_start_records_nothing_and_still_moves_the_baseline`, `a_plain_value_records_what_the_model_says`

### D30. An NRPN data entry with only its high byte

[Record section](transfer-from-seam-juce.md#d30-an-nrpn-data-entry-with-only-its-high-byte)

- `crates/learn/src/capture.rs`: module, `Resolution::Coarse`, `Stager::offer`, `Arm::capture`
- `crates/learn/src/lib.rs`: module
- `crates/learn/src/mappings.rs`: `Mapping::coarse`, `Learn::poll`
- `crates/learn/src/parse.rs`: module, `Value::Nrpn`, `Value::normalized`
- `crates/learn/src/saved.rs`: module
- test `crates/learn/tests/capture.rs`: module, `an_nrpn_high_byte_waits_for_its_low_byte_through_polls_with_no_block_between`, `an_nrpn_high_byte_alone_learns_as_coarse_after_a_tick_or_a_second_high_byte`, `a_newer_message_clears_a_high_byte_still_waiting`
- test `crates/learn/tests/dispatch.rs`: module, `an_nrpn_assembles_and_a_high_byte_alone_reads_as_midi_1_says`, `a_coarse_binding_reads_a_high_byte_as_seven_bit_and_ignores_low_bytes`, `a_data_entry_low_byte_sent_first_waits_for_its_high_byte`
- test `crates/learn/tests/saved.rs`: module, `the_section_is_schema_1_sorted_by_key_with_the_identity_curve_left_out`

### D31. What identifies a parameter to hosts

[Record section](transfer-from-seam-juce.md#d31-what-identifies-a-parameter-to-hosts)

- `crates/bridge/src/typescript.rs`: `param_map`
- `crates/host/src/cabi/abi.rs`: `SeamParamInfo::host_id`
- `crates/host/src/cabi/juce.rs`: `juce_param_id`
- `crates/host/src/plugin.rs`: `PARAMS`
- `crates/host/src/vst3/mod.rs`: module
- `crates/params/src/decl.rs`: `ParamId`
- `crates/params/src/host_id.rs`: module
- `crates/params/src/lib.rs`: module
- `crates/params/src/set.rs`: `DeclError::ReservedHostId`, `DeclError::HostIdCollision`, `ParamSet::host_id`
- test `crates/params/tests/set.rs`: `a_host_id_is_a_fixed_hash_of_the_key`, `a_key_whose_host_id_is_reserved_or_taken_is_refused`
- test `examples/eq/tests/eq.rs`: `every_declaration_is_accepted_with_its_own_host_id`

### D32. When the host hears a gesture begin

[Record section](transfer-from-seam-juce.md#d32-when-the-host-hears-a-gesture-begin)

- `crates/params/src/ui.rs`: module, `UiState::open`, `UiParams::begin`
- test `crates/params/tests/watch.rs`: `a_learned_write_mid_touch_moves_the_dsp_but_waits_for_the_watch`
- test `crates/params/tests/writes.rs`: module, `a_touch_reaches_the_host_before_anything_changes`

### D33. Undo during a user gesture

[Record section](transfer-from-seam-juce.md#d33-undo-during-a-user-gesture)

- `crates/params/src/ui.rs`: `UiParams::user_gesture_open`, `UiParams::replay`
- `crates/undo/src/history.rs`: module, `History::ready_to_replay`
- `crates/undo/src/lib.rs`: module, `HistoryError::GestureOpen`
- test `crates/params/tests/writes.rs`: module, `undo_can_tell_when_a_user_gesture_is_open`
- test `crates/undo/src/history/tests.rs`: `undo_waits_while_a_user_gesture_is_open`
- test `crates/undo/tests/enroll.rs`: module, `undo_waits_while_a_plain_value_gesture_is_open`

### D34. A sync while a gesture is open

[Record section](transfer-from-seam-juce.md#d34-a-sync-while-a-gesture-is-open)

- `crates/params/src/origin.rs`: `Origin`
- `crates/params/src/watch.rs`: `Watch`

### D35. A reserved NRPN block

[Record section](transfer-from-seam-juce.md#d35-a-reserved-nrpn-block)

- `crates/learn/src/reservation.rs`: `NrpnReservation`

### D36. The order one undo tick records

[Record section](transfer-from-seam-juce.md#d36-the-order-one-undo-tick-records)

- `crates/params/src/clock.rs`: module, `RestoreClock`, `RestoreReader::sequence`, `Stamped`
- `crates/params/src/events.rs`: `GestureRecord::sequence`
- `crates/params/src/lib.rs`: module
- `crates/params/src/ui.rs`: `GestureRecords::take`
- `crates/undo/src/enroll.rs`: module, `History::tick`
- `crates/undo/src/lib.rs`: module
- test `crates/undo/tests/enroll.rs`: module, `one_tick_records_steps_in_the_order_they_happened`

### D37. Whether the history keeps enrolled values alive

[Record section](transfer-from-seam-juce.md#d37-whether-the-history-keeps-enrolled-values-alive)

- `crates/params/src/lib.rs`: module
- `crates/params/src/list.rs`: `List::take_tracker`, `ListTracker`
- `crates/params/src/value.rs`: `Value::take_tracker`, `Tracker`
- `crates/undo/src/enroll.rs`: module, `History::enroll`, `History::tick`
- `crates/undo/src/lib.rs`: module
- `crates/undo/src/list.rs`: `History::enroll_list`
- test `crates/undo/src/list.rs`: `the_tick_forgets_a_list_whose_last_handle_dropped`
- test `crates/undo/tests/enroll.rs`: `a_forgotten_or_dropped_value_replays_as_nothing`
- test `crates/undo/tests/list.rs`: module, `every_element_drops_exactly_once_whatever_drops_first`

### D38. The origin of a newest write

[Record section](transfer-from-seam-juce.md#d38-the-origin-of-a-newest-write)

- `crates/params/src/lib.rs`: module
- `crates/params/src/origin.rs`: `Origin::ALL`, `Reading`, `Reading::origin`
- `crates/params/src/set.rs`: `ParamSet`, `ParamSet::read`, `ParamSet::values`
- `crates/params/src/set/value_cell.rs`: module, `Packed`, `Packed::new`, `Packed::origin`, `ValueCell`
- test `crates/params/src/origin.rs`: `every_origin_reads_back_from_its_number_and_no_other_byte_reads`
- test `crates/params/src/set/tests.rs`: `loom_racing_writers_leave_one_writers_value_with_its_origin`
- test `crates/params/src/set/value_cell.rs`: `a_word_gives_back_the_value_and_the_origin_it_was_made_with`
- `crates/undo/src/lib.rs`: module
- test `crates/params/tests/read.rs`: module, `a_plain_value_reads_its_count_its_value_and_that_values_origin_together`, `a_list_reads_what_its_closure_makes_with_the_count_and_the_origin`, `a_value_derived_from_a_reading_passes_on_the_origin_of_the_value_it_used`
- test `crates/params/tests/writes.rs`: `every_write_leaves_its_origin_beside_the_change_count`
- test `crates/undo/tests/enroll.rs`: module, `a_change_derived_from_a_parameter_or_a_list_records_nothing`

### D39. A load between a replay's check and its write

[Record section](transfer-from-seam-juce.md#d39-a-load-between-a-replays-check-and-its-write)

- `crates/params/src/clock.rs`: `RestoreClock`, `RestoreWriter::hold_loads`, `RestoreReader::hold_loads`
- `crates/params/src/lib.rs`: module
- `crates/params/src/program.rs`: `Load`, `Load::restore`
- `crates/undo/src/history.rs`: `Step::writes_parameters`, `History::undo`, `History::replay`
- `crates/undo/src/lib.rs`: module
- test `crates/undo/src/enroll.rs`: `a_parameter_step_holds_loads_off_and_a_value_step_does_not`
- test `crates/undo/src/history/tests.rs`: `a_step_that_writes_parameters_holds_loads_off_and_no_other_does`
- test `crates/undo/tests/restore.rs`: module, `a_load_waits_while_a_replay_holds_loads_off`, `a_load_racing_an_undo_always_ends_on_the_loaded_value`, `a_step_recorded_after_a_load_began_never_replays_over_its_write`

### D40. One undo history per UiParams

[Record section](transfer-from-seam-juce.md#d40-one-undo-history-per-uiparams)

- `crates/params/src/lib.rs`: module
- `crates/params/src/ui.rs`: `UiParams::take_gesture_records`
- `crates/undo/src/history.rs`: `Shared::records`, `History`, `History::new`
- `crates/undo/src/lib.rs`: module, `HistoryError::HistoryExists`
- test `crates/params/tests/writes.rs`: `there_is_one_ui_handle_and_one_drain_at_a_time`
- test `crates/undo/tests/enroll.rs`: `a_value_is_enrolled_once_in_every_build`

### D41. A list step replaying against a waiting verb

[Record section](transfer-from-seam-juce.md#d41-a-list-step-replaying-against-a-waiting-verb)

- `crates/params/src/list.rs`: `TrackedList`, `TrackedList::current`
- `crates/params/src/value.rs`: `Value::write`
- `crates/undo/src/list.rs`: `MoveStep`
- test `crates/undo/src/list.rs`: `a_verb_waiting_when_a_step_replays_makes_the_lists_steps_stale_for_good`
- test `crates/undo/tests/enroll.rs`: `a_worker_write_during_an_undos_tick_never_records_over_the_replay`
- test `crates/undo/tests/list.rs`: `every_guard_on_a_steps_handle_makes_it_a_no_op`

### D42. Order across ticks

[Record section](transfer-from-seam-juce.md#d42-order-across-ticks)

- `crates/params/src/clock.rs`: `RestoreReader::sequence`
- `crates/params/src/list.rs`: `ListTracker::take_before`, `TrackedList`
- `crates/params/src/value.rs`: `Tracker::take_before`
- `crates/undo/src/enroll.rs`: `Polled::poll`, `History::tick`
- `crates/undo/src/history.rs`: `History::same_clock`, `History::cut`
- `crates/undo/src/lib.rs`: module, `HistoryError::OtherClock`
- test `crates/undo/tests/enroll.rs`: `changes_made_during_a_tick_wait_for_the_next_one_in_order`, `a_value_or_list_made_with_another_clock_is_refused`

### D43. A derived change records nothing

[Record section](transfer-from-seam-juce.md#d43-a-derived-change-records-nothing)

- `crates/params/src/list.rs`: `List`, `List::derived`
- `crates/params/src/value.rs`: `Value`, `Value::set_from`
- `crates/undo/src/lib.rs`: module
- test `crates/undo/tests/enroll.rs`: `a_write_derived_from_a_change_records_nothing_and_follows_its_replays`, `only_a_plain_values_own_writes_record`, `a_change_derived_from_a_parameter_or_a_list_records_nothing`, `a_list_kept_in_step_with_a_value_records_nothing_and_follows_its_replays`

### D44. A participant that changes while its owner holds it

[Record section](transfer-from-seam-juce.md#d44-a-participant-that-changes-while-its-owner-holds-it)

- `crates/learn/src/mappings/session.rs`: `Learn::attach`
- `crates/session/src/lib.rs`: module
- `crates/session/src/ui.rs`: module, `Status::HeldUp`, `UiState::capture`, `SessionUi::tick`, `Registration::changed`
- test `crates/learn/tests/saved.rs`: `attached_to_a_session_learn_saves_each_change_and_restores_through_it`, `a_held_entry_landing_or_dropping_at_registration_reaches_the_session`
- test `crates/session/tests/session.rs`: `a_change_told_from_the_participants_own_method_saves_after_the_next_tick`, `a_panicking_capture_reaches_the_caller_of_changed_but_a_tick_contains_it`, `a_tick_inside_a_held_up_capture_captures_the_others_still_held_up`

### D45. Registrations when a SessionUi drops

[Record section](transfer-from-seam-juce.md#d45-registrations-when-a-sessionui-drops)

- `crates/session/src/session.rs`: `Session::take_ui`
- `crates/session/src/ui.rs`: module, `UiState::drop`, `SessionUi`, `Registration`, `Registration::drop`
- test `crates/session/tests/session.rs`: `dropping_the_session_ui_ends_its_registrations_and_keeps_their_sections`

### D46. A derived write and a plain value's open gesture

[Record section](transfer-from-seam-juce.md#d46-a-derived-write-and-a-plain-values-open-gesture)

- `crates/params/src/value.rs`: `Commit`, `Value::write`
- test `crates/undo/tests/enroll.rs`: `a_derived_write_inside_a_drag_follows_its_origin`

### D47. When a lone NRPN high byte settles as coarse

[Record section](transfer-from-seam-juce.md#d47-when-a-lone-nrpn-high-byte-settles-as-coarse)

- `crates/learn/src/capture.rs`: module, `Slots::begun`, `Slots::ended`, `Stager::begin_block`, `Stager::end_block`, `Stager::offer`, `Arm::capture`, `Arm::has_waited`, `Unsettled`
- `crates/learn/src/audio.rs`: `LearnAudio::process`
- `crates/learn/src/mappings.rs`: `Learn::poll`
- test `crates/learn/tests/capture.rs`: `an_nrpn_high_byte_alone_learns_as_coarse_after_a_tick_or_a_second_high_byte`, `an_nrpn_high_byte_waits_for_its_low_byte_through_polls_with_no_block_between`
- test `crates/learn/src/capture/tests.rs`: `loom_a_low_byte_in_the_next_block_is_never_missed`

### D48. A plain Value as a learn Target

[Record section](transfer-from-seam-juce.md#d48-a-plain-value-as-a-learn-target)

- `crates/learn/src/target.rs`: `Target::held`

### D49. A participant writing a parameter during apply

[Record section](transfer-from-seam-juce.md#d49-a-participant-writing-a-parameter-during-apply)

- `crates/params/src/ui.rs`: `UiState::write`
- test `crates/params/tests/writes.rs`: `a_user_write_while_a_restore_applies_is_refused_in_debug`

### D50. Learn's audio side per activation

[Record section](transfer-from-seam-juce.md#d50-learns-audio-side-per-activation)

- `crates/learn/src/mappings.rs`: `Learn::replace_audio`
- test `crates/learn/tests/saved.rs`: `a_fresh_audio_side_reads_the_current_mappings_and_the_old_one_stops`

### D51. Whether a plugin must be Send

[Record section](transfer-from-seam-juce.md#d51-whether-a-plugin-must-be-send)

- `crates/host/src/boundary.rs`: module, `MainThread`
- `crates/host/src/cabi/mod.rs`: `plugin_moving`, `Table`
- `crates/host/src/engine/mod.rs`: `Engine::new`, `Engine::new_unbound`
- `crates/host/src/plugin.rs`: `Plugin`, `Plugin::new`
- `crates/host/src/state.rs`: `SavedState::set`
- test `crates/host/src/cabi/tests.rs`: `the_exported_table_lends_a_send_plugin_to_any_thread_and_keeps_any_other_where_it_was_made`
- test `crates/host/tests/plugin.rs`: module, `a_plugin_owning_main_thread_state_saves_and_loads_from_another_thread`

### D52. Who makes the RestoreClock

[Record section](transfer-from-seam-juce.md#d52-who-makes-the-restoreclock)

- `crates/host/src/state.rs`: `SavedState::new`, `SavedState::set`
- `crates/learn/src/mappings/session.rs`: `Learn::attach`
- `crates/params/src/clock.rs`: `RestoreClock`, `RestoreClock::new`
- `crates/params/src/lib.rs`: module
- `crates/params/src/list.rs`: `List::new`
- `crates/params/src/set.rs`: `ParamSet::clock`
- `crates/params/src/value.rs`: `Value::new`
- `crates/session/src/session.rs`: `Session::new`
- `crates/session/src/ui.rs`: `SessionUi::uses`
- test `crates/host/tests/state.rs`: `a_saved_state_refuses_a_session_made_from_another_set`, `a_load_marks_the_clock_every_history_of_the_set_reads`
- test `crates/learn/tests/saved.rs`: `attaching_to_a_session_made_from_another_set_is_refused`
- test `crates/params/tests/clock.rs`: `each_set_makes_its_own_clock_and_hands_out_only_that_one`
- test `crates/undo/tests/enroll.rs`: `a_value_is_enrolled_once_in_every_build`, `a_value_or_list_made_with_another_clock_is_refused`

### D53. What ticks a plugin with no editor open

[Record section](transfer-from-seam-juce.md#d53-what-ticks-a-plugin-with-no-editor-open)

- `adapters/juce/SeamPluginProcessor.h`: module
- `bench/hosted/hosted.cpp`: `pumpMessagesUntil`
- `crates/host/Cargo.toml`: `[features]`
- `crates/host/src/cabi/abi.rs`: `SeamPlugin::idle`
- `crates/host/src/cabi/mod.rs`: module
- `crates/host/src/plugin.rs`: `Plugin::idle`
- `crates/host/src/engine/mod.rs`: `Engine::start_timer`, `Engine::ticker`
- `crates/host/src/engine/timer.rs`: module, `Timer`, `abandon`
- `crates/host/src/vst3/component.rs`: `Component::initialize`
- `crates/host/src/vst3/mod.rs`: module, `export_vst3`
- test `crates/host/src/engine/timer/tests.rs`: `the_timer_ticks_through_the_threads_loop_until_dropped`, `an_abandoned_timer_ticks_no_more`, `an_abandoned_timer_s_window_stops_it_at_its_next_message`
- test `crates/host/tests/plugin.rs`: `a_mapping_learned_after_the_session_ticks_waits_a_tick_for_the_save`

### D54. Detaching a learn target

[Record section](transfer-from-seam-juce.md#d54-detaching-a-learn-target)

- `crates/learn/src/mappings.rs`: `Learn::held`, `Learn::register`, `Learn::detach`
- test `crates/learn/tests/targets.rs`: `a_detached_target_keeps_its_mapping_until_its_key_registers_again`, `a_target_panicking_as_it_detaches_leaves_its_mapping_held_and_learn_usable`, `a_detached_unmapped_target_leaves_nothing_held`

### D55. A participant whose apply panics

[Record section](transfer-from-seam-juce.md#d55-a-participant-whose-apply-panics)

- `crates/session/src/lib.rs`: `Participant::apply`
- `crates/session/src/ui.rs`: `Status::Faulted`, `UiState::capture`, `Registration::changed`
- test `crates/session/tests/session.rs`: `a_participant_whose_apply_panicked_is_held_until_an_apply_succeeds`, `a_failed_apply_does_not_hold_its_participant`

### D56. The hash that names a parameter

[Record section](transfer-from-seam-juce.md#d56-the-hash-that-names-a-parameter)

- `crates/params/src/lib.rs`: module
- test `crates/params/tests/set.rs`: `the_reserved_range_is_pinned_at_both_edges`

### D57. The lattice of a stepped range

[Record section](transfer-from-seam-juce.md#d57-the-lattice-of-a-stepped-range)

- `bench/juce-eq/SeamRange.h`: module
- `crates/params/src/decl.rs`: `Range`, `Range::lattice`, `Lattice`
- `crates/params/src/meta.rs`: `lattice`
- `examples/eq/src/dsp.rs`: module
- test `crates/params/src/decl/tests.rs`: `a_value_reads_back_as_the_decimal_it_was_written_as`, `a_lattice_counts_in_its_finest_decimal_place`, `a_range_made_in_a_const_holds_the_lattice_a_range_made_at_run_time_does`
- test `crates/params/tests/snap.rs`: `nearest_legal`, `snapping_gives_the_nearest_legal_value`, `every_decimal_point_lands_on_its_nearest_f32`, `a_decimal_step_lands_on_its_decimals`

### D58. What the plugin trait tells every host

[Record section](transfer-from-seam-juce.md#d58-what-the-plugin-trait-tells-every-host)

- `crates/host/src/info.rs`: `PluginInfo`, `Feature`, `Layout`
- `crates/host/src/plugin.rs`: `Plugin::latency`, `Plugin::tail`, `Plugin::deactivate`

### D59. The shape of the VST3 exporter

[Record section](transfer-from-seam-juce.md#d59-the-shape-of-the-vst3-exporter)

- `crates/host/src/boundary.rs`: `MainThread`, `AudioSlot`
- `crates/host/src/engine/automation.rs`: module, `Point`, `Points::queue`, `Points::apply`
- `crates/host/src/engine/mod.rs`: module, `Engine`, `Engine::new`, `Engine::release`, `Engine::leave`, `Engine::controller_write`, `Engine::set_processing`, `Engine::tick`
- `crates/host/src/engine/timer.rs`: `abandon`
- `crates/host/src/vst3/component.rs`: module, `Component`, `Component::initialize`, `read_changes`
- test `crates/host/src/engine/tests.rs`: module, `a_plugin_created_on_one_thread_runs_on_the_thread_that_calls_it_first`, `a_controller_write_while_processing_is_held_off_only_where_the_host_sends_it_again`, `created_on_one_thread_driven_from_a_second_and_played_on_a_third`
- test `crates/host/src/vst3/tests.rs`: `the_plugin_is_built_at_initialize_on_the_thread_that_calls_it`, `a_release_on_another_thread_leaks_the_plugin_drops_the_processor_and_counts`, `a_processor_that_panics_as_it_drops_on_another_thread_is_caught_there`, `a_release_inside_a_tick_leaks_what_the_tick_still_uses`, `a_tick_nested_inside_a_tick_does_nothing`, `the_last_point_of_a_queue_wins`, `every_point_keeps_its_offset_and_each_queue_lands_its_last`

### D60. A panic inside a host's call

[Record section](transfer-from-seam-juce.md#d60-a-panic-inside-a-hosts-call)

- `crates/host/src/boundary.rs`: `guard`
- `crates/host/src/cabi/mod.rs`: `export_cabi`
- `crates/host/src/engine/process.rs`: `Audio`, `run_held`, `Engine::process`, `Engine::play`
- `crates/host/src/faults.rs`: module, `Faults`, `Faults::guard`, `boundary_faults`
- `crates/host/src/vst3/mod.rs`: `export_vst3`
- test `crates/host/src/cabi/tests.rs`: `a_panic_in_activate_or_idle_is_answered`
- test `crates/host/src/engine/process.rs`: `loom_a_panicking_block_racing_an_activation_marks_only_its_own_processor`
- test `crates/host/src/engine/tests.rs`: `an_activation_while_active_rebuilds_and_recovers_a_panicked_processor`
- test `crates/host/src/vst3/tests.rs`: `a_panicking_entry_point_answers_an_error`, `an_inactive_or_panicking_processor_plays_silence`

### D61. Who flushes denormals

[Record section](transfer-from-seam-juce.md#d61-who-flushes-denormals)

- `adapters/juce/SeamPluginProcessor.h`: module
- `bench/juce-eq/PluginProcessor.cpp`: `JuceEq::processBlock`
- `bench/juce-eq/PluginProcessor.h`: module
- `crates/host/src/engine/process.rs`: `Engine::play`

### D62. The C interface for the JUCE adapter

[Record section](transfer-from-seam-juce.md#d62-the-c-interface-for-the-juce-adapter)

- `crates/host/include/seam.h`: module
- `crates/host/src/boundary.rs`: `MainThread::bound`, `MainThread::unbound`, `MainThread::lend`, `MainThread::lend_or_wait`, `MainThread::is_owner`, `MainThread::owned`, `here`, `Wait`, `Lent`, `Refused`
- `crates/host/src/cabi/abi.rs`: `SeamPlugin`, `SeamHost`
- `crates/host/src/cabi/instance.rs`: `Instance`, `Callbacks`
- `crates/host/src/cabi/mod.rs`: module, `plugin`, `plugin_moving`, `Table`, `BoundAtCreation`, `create_unbound`, `destroy`, `text`, `export_cabi`
- `crates/host/src/engine/mod.rs`: `LOAN_WAIT`, `Engine`, `Engine::new_unbound`, `Engine::main`, `Engine::idle`, `Engine::host_write`, `Engine::release`, `Engine::leak_refused`
- test `crates/host/src/boundary/loom_models.rs`: `loom_unbound_state_is_lent_to_one_thread_at_a_time`
- test `crates/host/src/boundary/tests.rs`: `unbound_state_is_lent_to_any_thread_for_good`, `unbound_state_lent_out_is_refused_elsewhere_and_lent_again_on_its_thread`, `unbound_state_drops_on_whichever_thread_drops_it`, `a_waiting_loan_takes_unbound_state_once_its_holder_lets_go`, `a_waiting_loan_is_refused_once_its_patience_runs_out`, `bound_state_on_another_thread_is_refused_without_a_wait`
- test `crates/host/src/cabi/tests.rs`: `the_exported_table_lends_a_send_plugin_to_any_thread_and_keeps_any_other_where_it_was_made`, `a_moving_plugin_takes_main_thread_calls_from_any_thread_however_many_ticks_ran`, `a_tick_while_an_activation_runs_elsewhere_skips_without_a_fault`, `an_activation_while_a_tick_runs_elsewhere_waits_and_takes_it`, `a_main_thread_call_held_off_past_the_wait_is_refused_and_counted`, `a_destroy_while_another_thread_ticks_leaks_the_instance_without_waiting`, `a_moving_plugin_called_from_inside_its_own_tick_shares_that_tick_s_loan`, `an_instance_bound_to_no_thread_starts_no_timer`, `a_destroy_on_another_thread_leaks_the_plugin_drops_the_processor_and_counts`, `a_destroy_inside_a_tick_ends_the_ticks_calls_to_the_host`, `text_is_cut_at_a_character`
- test `crates/host/src/engine/tests.rs`: `a_plugin_created_on_one_thread_runs_on_the_thread_that_calls_it_first`

### D63. Keeping the C header true

[Record section](transfer-from-seam-juce.md#d63-keeping-the-c-header-true)

- `.github/workflows/ci.yml`: `header:`
- `crates/host/src/cabi/abi.rs`: `CType`, `CChar`
- `crates/host/src/cabi/header.rs`: `header`
- test `crates/host/src/cabi/tests.rs`: `the_header_declares_every_struct_and_asserts_its_layout`, `the_header_spells_each_type_as_c_does`, `the_header_s_wait_is_the_loan_wait_s_own`
- test `xtask/tests/drift.rs`: module

### D64. How the JUCE adapter builds a plugin

[Record section](transfer-from-seam-juce.md#d64-how-the-juce-adapter-builds-a-plugin)

- `.github/workflows/juce-adapter.yml`: module
- `adapters/juce/SeamJuce.cmake`: `seam_add_juce_plugin`, `_seam_apple_build`
- `crates/host/src/cabi/juce.rs`: `juce_settings`

### D65. What a host sees through JUCE

[Record section](transfer-from-seam-juce.md#d65-what-a-host-sees-through-juce)

- `.github/workflows/juce-adapter.yml`: module
- `adapters/juce/SeamPluginProcessor.cpp`: `PluginProcessor::processBlockBypassed`, `PluginProcessor::reportLatency`, `PluginProcessor::timerCallback`, `getValueForText`
- `adapters/juce/SeamPluginProcessor.h`: module, `getProgramName`
- `crates/host/src/cabi/juce.rs`: `juce_param_id`
- `crates/host/src/plugin.rs`: `Plugin`

### D66. Bytes after the block, and a failed load

[Record section](transfer-from-seam-juce.md#d66-bytes-after-the-block-and-a-failed-load)

- `crates/host/src/cabi/mod.rs`: `load`
- `crates/host/src/state.rs`: `SavedState::load`
- `crates/host/src/vst3/component.rs`: `Component::setState`
- test `crates/host/src/engine/tests.rs`: `a_block_padded_with_nuls_loads_and_one_followed_by_anything_else_fails`
- test `crates/host/tests/hostile.rs`: `cut_flipped_and_padded_blocks_keep_every_promise`

### D67. Checking the audio-thread locking rule

[Record section](transfer-from-seam-juce.md#d67-checking-the-audio-thread-locking-rule)

- `crates/host/src/cabi/mod.rs`: `set`, `reset`
- `crates/host/src/engine/process.rs`: `Engine::process`
- `crates/host/src/vst3/component.rs`: `setProcessing`
- `crates/learn/src/audio.rs`: `LearnAudio`
- `crates/params/src/audio_thread.rs`: module, `enter`, `AudioThreadGuard`, `is_here`, `assert_not_here`
- `crates/params/src/clock.rs`: `RestoreClock::hold_loads`
- `crates/params/src/host_events.rs`: `HostEventDrain`
- `crates/params/src/list.rs`: `List`
- `crates/params/src/program.rs`: `Load::restore`
- `crates/params/src/set.rs`: `AudioParams`
- `crates/params/src/value.rs`: `Value`
- `crates/session/src/session.rs`: `Session`
- test `crates/host/src/engine/tests.rs`: `the_block_runs_marked_as_the_audio_thread_and_no_processor_drops_there`
- test `crates/learn/tests/capture.rs`: `dropping_the_audio_side_is_silent_off_the_audio_thread`, `dropping_the_audio_side_is_refused_on_the_audio_thread`
- test `crates/params/tests/audio_thread.rs`: module, `the_mark_belongs_to_its_thread`, `every_checked_call_is_silent_off_the_audio_thread`, `a_panic_while_unwinding_is_not_made_worse`, `the_guard_is_zero_sized_and_nothing_is_ever_refused`
- test `crates/session/tests/audio_thread.rs`: module

### D68. Audit items that wait for their code

[Record section](transfer-from-seam-juce.md#d68-audit-items-that-wait-for-their-code)

- `crates/bridge/src/lib.rs`: module
- `crates/host/src/engine/automation.rs`: module

### D69. Where a continuous range is checked

[Record section](transfer-from-seam-juce.md#d69-where-a-continuous-range-is-checked)

- `crates/params/src/decl.rs`: `Range`, `Range::new`, `RangeError`, `Kind::continuous`
- test `crates/params/tests/set.rs`: `a_range_is_refused_unless_it_can_be_snapped_to_and_converted`

### D70. Learn's reading of MIDI 1.0 pairs

[Record section](transfer-from-seam-juce.md#d70-learns-reading-of-midi-10-pairs)

- `crates/learn/src/capture.rs`: module, `Stager::pair_low_byte`, `Stager::newest`, `Stager::offer`, `Resolution::Waiting`, `Arm::capture`, `is_low_byte`
- `crates/learn/src/lib.rs`: module
- `crates/learn/src/mappings.rs`: `Learn::poll`
- `crates/learn/src/source.rs`: `Control::Cc`, `Control::PARSER_OWNED`
- `crates/learn/src/parse.rs`: module, `Channel::owned`, `Channel::address_byte`, `Channel::unpaired`, `Channel::reset`, `Channel::address_changed`, `RESET_ALL_CONTROLLERS`, `NULL_NUMBER`
- test `crates/learn/src/parse.rs`: `the_parser_agrees_with_a_model_of_midi_1_0`, `model`, `begins_an_address`
- test `crates/learn/tests/capture.rs`: `a_14_bit_controller_pair_learns_its_high_byte`, `a_controller_32_to_63_learns_itself_a_tick_later_unless_its_high_byte_came_just_before`, `an_arm_between_a_pairs_halves_waits_through_polls_and_learns_the_high_byte_that_follows`, `a_capture_held_back_between_a_high_byte_and_its_low_byte_stops_the_pairing`, `data_increment_and_decrement_are_never_learned`
- test `crates/learn/tests/dispatch.rs`: `the_null_parameter_number_selects_nothing`, `a_data_entry_low_byte_sent_first_waits_for_its_high_byte`, `a_new_address_drops_a_waiting_low_byte`, `a_new_address_high_byte_forgets_the_old_low_byte`, `reset_all_controllers_forgets_the_address`, `an_nrpn_address_selects_its_number_whichever_byte_comes_first`, `juces_rpn_generator_reaches_its_nrpn_with_or_without_the_low_byte`, `a_lone_high_byte_never_pairs_with_a_low_byte_from_before_a_data_entry`, `data_increment_and_decrement_use_the_address_and_drive_nothing`, `the_parser_owned_controllers_never_bind`

### D71. Learn after an unreadable midiLearn section

[Record section](transfer-from-seam-juce.md#d71-learn-after-an-unreadable-midilearn-section)

- `crates/learn/src/mappings/session.rs`: `Learn::restore`
- test `crates/learn/tests/saved.rs`: `a_section_of_another_schema_is_refused_and_clears_the_routing`, `a_session_restore_of_another_schema_stops_the_old_routing_and_keeps_the_section`, `a_refused_section_reports_no_change_to_the_session`
- test `crates/learn/tests/targets.rs`: `a_target_panicking_as_a_restore_closes_its_burst_leaves_learn_usable`

### D72. How much a host may ask

[Record section](transfer-from-seam-juce.md#d72-how-much-a-host-may-ask)

- `crates/host/src/audio.rs`: `MAX_FRAMES`
- `crates/host/src/cabi/abi.rs`: `MAX_FRAMES`
- `crates/host/src/engine/automation.rs`: module, `POINTS_PER_PARAMETER`, `MAX_SKIPPED_QUEUES`, `Automation::new`, `Points::queue`, `Points::walking`, `Points::skip`
- `crates/host/src/engine/mod.rs`: `MAX_EVENTS`, `Setup::new`
- `crates/host/src/state.rs`: `MAX_STATE_BYTES`, `SavedState::load`
- `crates/host/src/vst3/component.rs`: `read_changes`
- `crates/host/src/vst3/stream.rs`: `read_all`
- test `crates/host/src/cabi/tests.rs`: `an_activation_past_the_block_size_limit_builds_for_the_limit`
- test `crates/host/src/engine/automation.rs`: `a_queue_whose_reads_fail_is_read_no_further_than_the_block_has_room`, `a_first_queue_whose_last_read_fails_still_wins_over_a_repeat`, `while_another_block_holds_the_room_a_repeat_stands_and_takes_a_place`, `a_queue_given_no_room_lands_its_last_point_and_counts_only_the_rest_dropped`
- test `crates/host/src/vst3/tests.rs`: `a_host_claiming_more_queues_than_parameters_is_walked_no_further`, `queues_for_ids_the_plugin_does_not_know_never_starve_its_own`, `a_block_moving_every_proxy_of_learn_s_grid_still_reaches_the_plugin_s_own_queue`, `a_parameter_in_two_queues_takes_one_parameter_s_place_and_its_first_queue_wins`, `the_first_queue_for_a_parameter_wins_wherever_the_repeat_comes`, `a_host_claiming_millions_of_queues_it_does_not_know_is_walked_a_bounded_way`, `a_queue_claiming_millions_of_points_it_cannot_give_is_read_as_far_as_there_is_room`, `a_queue_claiming_millions_of_points_is_read_as_far_as_there_is_room`, `a_setup_with_no_block_size_keeps_its_rate_and_one_past_the_limit_is_built_for_the_limit`
- test `crates/host/tests/state.rs`: `a_block_past_the_limit_is_refused_unread`
- test `examples/eq/tests/overflow.rs`: `a_block_with_more_points_than_the_room_allocates_nothing_and_lands_each_last_point`

### D73. What an activation makes of an open setup

[Record section](transfer-from-seam-juce.md#d73-what-an-activation-makes-of-an-open-setup)

- `crates/host/src/cabi/mod.rs`: `activate`
- `crates/host/src/engine/mod.rs`: `Setup::new`, `Setup::FALLBACK`, `Engine::activate`, `Engine::deactivate`
- `crates/host/src/vst3/component.rs`: `setupProcessing`, `setActive`
- test `crates/host/src/cabi/tests.rs`: `an_activation_with_no_block_size_takes_the_fallbacks`, `an_activation_drops_a_reset_asked_of_the_processor_it_replaces`
- test `crates/host/src/engine/tests.rs`: `an_activation_while_active_rebuilds_and_recovers_a_panicked_processor`, `a_reset_runs_before_the_next_block_unless_an_activation_replaced_the_processor`
- test `crates/host/src/vst3/tests.rs`: `a_setup_with_no_block_size_keeps_its_rate_and_one_past_the_limit_is_built_for_the_limit`

### D74. What export_vst3! refuses to build

[Record section](transfer-from-seam-juce.md#d74-what-export_vst3-refuses-to-build)

- `.github/workflows/ci.yml`: `linux:`
- `crates/host/src/engine/mod.rs`: `first_layout`
- `crates/host/src/vst3/component.rs`: `Component::new`
- `crates/host/src/vst3/component/buses.rs`: `arrangement`, `channels_of`
- `crates/host/src/vst3/mod.rs`: module, `check_layouts`, `export_vst3`
- test `crates/host/src/vst3/tests.rs`: `every_arrangement_offered_is_one_the_exporter_then_takes`, `a_bus_wider_than_stereo_is_refused_by_name`

### D75. Making a caught fault visible

[Record section](transfer-from-seam-juce.md#d75-making-a-caught-fault-visible)

- `crates/host/src/engine/mod.rs`: `Engine::report_faults`, `Engine::idle`, `Engine::release`, `Engine::refused`
- `crates/host/src/faults.rs`: module, `boundary_faults`, `Fault`, `Faults::record`, `Faults::report`, `record_fault`
- test `crates/host/src/cabi/tests.rs`: `a_main_thread_call_held_off_past_the_wait_is_refused_and_counted`
- test `crates/host/src/engine/tests.rs`: `a_fault_on_the_audio_path_is_reported_at_the_next_tick_once_per_instance_and_kind`, `a_release_reports_the_faults_no_tick_reported`
- test `crates/host/src/faults/tests.rs`: `an_instance_reports_each_kind_once_and_counts_every_fault`, `a_fault_recorded_on_another_thread_is_reported_on_this_one`, `every_fault_s_text_is_one_line_spaced_once`

### D76. A panic while edits are told to the host

[Record section](transfer-from-seam-juce.md#d76-a-panic-while-edits-are-told-to-the-host)

- `crates/params/src/host_events.rs`: `HostEventDrain::drain`, `HostEventDrain::finish`, `Requeue`
- test `crates/params/src/host_events.rs`: `loom_a_drain_racing_a_gesture_hears_one_balanced_bracket`
- test `crates/params/tests/writes.rs`: `a_panicking_emit_leaves_the_rest_for_the_next_drain`, `a_panicking_emit_keeps_a_gestures_end_for_the_next_drain`, `a_finish_whose_end_panics_keeps_the_bracket_open_for_the_next_finish`

### D77. How deep a session section may nest

[Record section](transfer-from-seam-juce.md#d77-how-deep-a-session-section-may-nest)

- `crates/host/src/state.rs`: `MAX_STATE_BYTES`, `VALUE_BYTES`, `SavedState::declared_bytes`, `SavedState::room_for_extra`, `SavedState::load`, `Unknown::bytes`
- `crates/session/src/session.rs`: `MAX_SECTION_DEPTH`, `MAX_STATE_BYTES`, `within_depth`, `accept`, `Kept`, `Claim::refused`, `State::room_for`, `State::register`, `State::keep`, `State::refuse_past_budget`, `Restored`, `Restored::bytes`, `Session::restore_within`, `Session::store`, `Session::set_budget`
- `crates/session/src/ui.rs`: `UiState::commit`, `Registration::refused`
- test `crates/host/src/state/tests.rs`: `a_declared_value_takes_at_most_value_bytes`
- test `crates/host/tests/state.rs`: `the_deepest_section_a_capture_takes_saves_a_block_that_loads_and_a_deeper_one_is_refused`, `a_capture_that_would_pass_the_load_limit_is_refused_and_the_block_saved_before_stays`, `the_largest_capture_saves_a_block_at_the_limit_that_loads_and_one_byte_more_is_refused`, `the_unknown_keys_a_load_kept_take_their_share_of_the_limit`, `a_block_within_the_limit_that_would_save_back_past_it_is_refused_and_changes_nothing`, `a_capture_taken_before_the_saved_state_is_made_is_judged_when_it_is_made`, `a_capture_taken_after_the_saved_state_is_made_is_refused_as_it_is_taken`, `a_restored_section_the_saved_state_drops_stays_dropped_through_its_apply`, `a_load_sets_its_budget_with_its_sections_so_an_apply_that_grows_is_refused_and_the_rest_stays`
- test `crates/session/tests/budget.rs`: module, `the_largest_capture_that_fits_fills_the_budget_exactly_and_one_byte_more_is_refused`, `escapes_in_keys_and_sections_are_counted_as_a_save_writes_them`, `every_section_shares_the_budget_those_no_participant_holds_included`, `a_registration_whose_capture_has_no_room_saves_nothing_under_its_key`, `a_restore_is_never_refused_and_a_capture_no_larger_than_what_it_replaces_is_taken`, `a_recapture_after_an_apply_that_has_no_room_keeps_the_restored_section`, `a_session_no_saved_state_set_a_budget_for_takes_sections_up_to_the_whole_block`, `a_lowered_budget_drops_the_sections_past_it_in_registration_order_and_refuses_them`, `a_lowered_budget_drops_a_restored_section_waiting_for_its_apply_and_the_apply_with_it`, `sections_no_participant_holds_are_judged_in_key_order_whatever_order_they_came_in`, `a_restored_section_a_lowered_budget_drops_is_not_applied_to_a_later_registration`
- test `crates/session/tests/depth.rs`: module, `a_section_at_the_depth_limit_saves_a_block_that_loads`, `a_section_past_the_limit_is_refused_and_the_previous_one_stays_saved`, `a_tree_of_sixty_two_levels_is_refused`, `a_restored_section_whose_recapture_is_too_deep_stays_as_restored`

### D78. Protocol versions

[Record section](transfer-from-seam-juce.md#d78-protocol-versions)

- `crates/bridge/protocol-shape.txt`: module
- `crates/bridge/src/protocol.rs`: `PROTOCOL_VERSION`, `PageMessage::decode`, `NativeMessage::Reject`
- `crates/bridge/tests/fixtures/messages.json`: module
- `page/src/client.ts`: `createClient`
- test `crates/bridge/src/protocol.rs`: `a_ready_of_a_newer_page_decodes_whatever_else_it_says`
- test `crates/bridge/tests/fixtures_messages.rs`: module, `what_the_native_side_refuses_it_refuses`, `what_the_native_side_tolerates_decodes_as_listed`
- test `crates/bridge/tests/protocol_version.rs`: `the_message_shapes_and_the_protocol_version_change_together`
- test `page/test/client.test.ts`: `a reject fails the client with the native side's version and reason`, `stops when a snapshot of another version fails the client`
- test `page/test/fixture.test.ts`: module

### D79. The page client before the native side answers

[Record section](transfer-from-seam-juce.md#d79-the-page-client-before-the-native-side-answers)

- `page/src/client.ts`: `READY_RETRY_MS`, `createClient`
- `page/src/hooks.ts`: `useCommand`
- test `page/test/client.test.ts`: `repeats until a snapshot arrives`, `does not repeat when the first ready is answered at once`, `does not repeat when the first ready is rejected at once`, `stops when the client is disposed`, `stops when a reject fails the client`
- test `page/test/useCommand.test.ts`: `a change of name starts clean, and a late reply to the old name stays out`

### D80. Upgrading an older saved block

[Record section](transfer-from-seam-juce.md#d80-upgrading-an-older-saved-block)

- `crates/host/src/state.rs`: `STATE_VERSION`, `LastLoad::versions`, `Migration`, `SavedState::with_migration`, `SavedState::save`, `SavedState::load`, `Unknown`
- test `crates/host/tests/state.rs`: `an_older_block_is_migrated_before_it_loads_and_one_that_cannot_be_changes_nothing`, `a_newer_builds_block_keeps_its_version_and_values_through_an_older_builds_save`, `a_fresh_state_and_one_that_loaded_an_older_block_save_their_own_version`, `unknown_top_level_keys_are_kept_saved_after_extra_in_key_order_and_replaced_by_the_next_load`, `a_block_within_the_limit_that_would_save_back_past_it_is_refused_and_changes_nothing`, `the_migration_runs_only_for_a_block_of_an_older_plugin_version_and_gets_that_version`

### D81. A registration made while a panic unwinds

[Record section](transfer-from-seam-juce.md#d81-a-registration-made-while-a-panic-unwinds)

- `crates/session/src/ui.rs`: `Passing`, `Passing::new`, `Passing::drop`, `SessionUi::register`, `SessionUi::tick`
- test `crates/session/tests/session.rs`: `a_pass_a_panic_escapes_is_owed_again_and_later_restores_still_apply`, `a_registration_made_while_unwinding_owes_no_pass`

### D82. When learn runs a target's own code

[Record section](transfer-from-seam-juce.md#d82-when-learn-runs-a-targets-own-code)

- `crates/learn/src/mappings.rs`: `Learn::unmap`
- `crates/learn/src/mappings/session.rs`: `Learn::restore`
- test `crates/learn/tests/targets.rs`: `a_target_panicking_as_a_restore_closes_its_burst_leaves_learn_usable`, `a_target_panicking_as_a_restore_leaves_the_audio_side_on_the_new_table`, `a_target_panicking_as_it_unmaps_leaves_the_table_published_and_saved`

### D83. A control's drag and one-shot set

[Record section](transfer-from-seam-juce.md#d83-a-controls-drag-and-one-shot-set)

- `page/src/hooks.ts`: `useParam`, `onDrag`, `setValue`
- test `page/test/useParam.test.ts`: `brackets one drag into begin, set..., end in order`, `setValue outside a drag sends only a set`

### D84. The saved block's two versions

[Record section](transfer-from-seam-juce.md#d84-the-saved-blocks-two-versions)

- `crates/host/src/state.rs`: `STATE_VERSION`, `SEAM_VERSION`, `PLUGIN_VERSION`, `Versions`, `Versions::newest`, `LastLoad::versions`, `SavedState::versions`, `SavedState::new`, `SavedState::save`, `SavedState::load`, `versions`, `whole`, `LoadError::BadVersion`, `LoadError::BadPluginVersion`
- test `crates/host/tests/state.rs`: `a_block_is_version_parameters_extra_in_that_order_with_each_value_typed_by_kind`, `each_version_saves_as_the_higher_of_the_blocks_and_this_builds_on_its_own`, `the_migration_runs_only_for_a_block_of_an_older_plugin_version_and_gets_that_version`, `a_block_without_a_plugin_version_is_at_0`, `a_save_writes_both_versions_first_whatever_order_the_block_held_them_in`, `a_version_past_u64s_range_or_written_as_a_float_still_loads`, `a_refused_block_says_which_check_it_failed`, `the_largest_capture_saves_a_block_at_the_limit_that_loads_and_one_byte_more_is_refused`
- test `crates/host/tests/hostile.rs`: `block`, `check`

### D85. A migration that cannot upgrade a block

[Record section](transfer-from-seam-juce.md#d85-a-migration-that-cannot-upgrade-a-block)

- `crates/host/src/state.rs`: `Migrate`, `Migration`, `SavedState::with_migration`, `SavedState::load`, `LoadError::MigrationFailed`, `LoadError::source`
- test `crates/host/tests/state.rs`: `a_migrations_error_fails_the_load_with_its_reason_and_changes_nothing`, `an_older_block_is_migrated_before_it_loads_and_one_that_cannot_be_changes_nothing`, `hostile_blocks_through_a_migration_keep_every_promise`

### D86. The AU codes

[Record section](transfer-from-seam-juce.md#d86-the-au-codes)

- `crates/host/src/info.rs`: `AuCodes`, `PluginInfo::au_codes`
- `crates/host/src/cabi/juce.rs`: `juce_settings`
- `examples/eq/seam-juce.cmake`: `SEAM_PLUGIN_CODE`, `SEAM_PLUGIN_MANUFACTURER_CODE`
- test `xtask/tests/drift.rs`: `every_generated_file_is_current`

### D87. The main bus in `ProcessSetup`

[Record section](transfer-from-seam-juce.md#d87-the-main-bus-in-processsetup)

- `crates/host/src/audio.rs`: `ProcessSetup::layout`, `Block::channels`
- `crates/host/src/engine/process.rs`: `Audio::new`
- test `crates/host/src/cabi/tests.rs`: `an_instrument_plays_with_no_input_or_a_narrower_one`, `activation_builds_a_processor_for_the_setup_and_layout`
- test `crates/host/src/vst3/tests.rs`: `layouts_follow_the_declaration`

### D88. A call a test host was refused

[Record section](transfer-from-seam-juce.md#d88-a-call-a-test-host-was-refused)

- `crates/host/src/vst3/test_host.rs`: `Refused`, `Refused::check`, `REFUSALS`, `TestHost::load`, `Blocks::send`
- `crates/host/src/cabi/test_host.rs`: `Refused`, `Refused::check`, `TestHost::activate`, `TestHost::load`
- test `crates/host/src/engine/tests.rs`: `Shell::Refused`, `a_block_padded_with_nuls_loads_and_one_followed_by_anything_else_fails`
- test `crates/host/src/vst3/tests.rs`: `the_host_writes_through_process_and_through_the_controller`, `layouts_follow_the_declaration`, `a_panicking_entry_point_answers_an_error`, `state_round_trips_through_streams_on_any_thread`, `setup_is_refused_while_active`, `a_setup_with_no_block_size_keeps_its_rate_and_one_past_the_limit_is_built_for_the_limit`
- test `crates/host/src/cabi/tests.rs`: `activation_builds_a_processor_for_the_setup_and_layout`, `a_panic_in_activate_or_idle_is_answered`, `the_saved_block_round_trips_and_a_stranger_changes_nothing`, `an_instrument_plays_with_no_input_or_a_narrower_one`

## Parameters

### P1. One store per parameter

[Record section](transfer-from-seam-juce.md#p1-one-store-per-parameter)

- `crates/params/src/lib.rs`: module
- `crates/params/src/set.rs`: module, `ParamSet`, `ParamSet::normalized`, `ParamSet::values`
- `crates/params/src/set/value_cell.rs`: module, `Packed::value`, `ValueCell`
- test `crates/params/src/set/tests.rs`: `loom_racing_writers_leave_one_writers_value_with_its_origin`
- test `crates/params/tests/set.rs`: module

### P2. Every write snaps, per kind

[Record section](transfer-from-seam-juce.md#p2-every-write-snaps-per-kind)

- `crates/params/src/meta.rs`: `ParamDecl::format_real`
- test `crates/host/tests/state.rs`: `save_load_save_gives_the_same_bytes_on_linear_skewed_and_stepped_ranges`
- test `crates/params/tests/snap.rs`: module, `a_stepped_range_snaps_to_the_nearest_lattice_point_or_its_end`, `a_host_reaches_both_ends`

### P3. A non-finite value never reaches the store

[Record section](transfer-from-seam-juce.md#p3-a-non-finite-value-never-reaches-the-store)

- `crates/params/src/decl.rs`: `Kind::snap`, `ParamDecl`
- test `crates/params/tests/snap.rs`: module

### P4. Defaults are legal values

[Record section](transfer-from-seam-juce.md#p4-defaults-are-legal-values)

- `crates/params/src/decl.rs`: `ParamDecl::default_value`
- test `crates/params/tests/snap.rs`: module

### P5. "Changed" means the snapped value differs from the store

[Record section](transfer-from-seam-juce.md#p5-changed-means-the-snapped-value-differs-from-the-store)

- `crates/params/src/ui.rs`: module, `ParamGesture::shown`, `UiParams`, `UiParams::would_change`
- test `crates/params/tests/writes.rs`: module, `a_click_that_changes_nothing_does_nothing`, `a_user_write_compares_with_the_value_the_gesture_shows`

### P6. Write origins decide every side effect

[Record section](transfer-from-seam-juce.md#p6-write-origins-decide-every-side-effect)

- `crates/params/src/audio_thread.rs`: module
- `crates/params/src/lib.rs`: module
- `crates/params/src/set.rs`: `AudioParams`
- `crates/params/src/ui.rs`: module
- test `crates/params/tests/writes.rs`: module, `a_user_write_while_a_restore_applies_is_refused_in_debug`

### P7. The host write: sanitise, clamp, convert, snap, store, publish

[Record section](transfer-from-seam-juce.md#p7-the-host-write-sanitise-clamp-convert-snap-store-publish)

- `adapters/juce/SeamPluginProcessor.cpp`: `PluginProcessor`
- `crates/host/src/cabi/abi.rs`: `SeamPlugin::set`
- `crates/host/src/cabi/mod.rs`: module
- `crates/host/src/vst3/mod.rs`: module
- `crates/params/src/decl.rs`: `ParamDecl::host_value`
- `crates/params/src/set.rs`: module, `ParamSet`, `ParamSet::read`, `ParamSet::store_then_publish`, `AudioParams`, `AudioParams::host_write`
- `examples/eq/examples/parity/cases.rs`: `normalized`
- test `crates/params/src/set/tests.rs`: `loom_a_read_never_has_a_count_newer_than_its_value`, `loom_a_user_write_and_a_sync_publish_their_count_and_request_after_their_value`
- test `crates/params/tests/alloc_gate.rs`: module
- test `crates/params/tests/set.rs`: module, `a_host_write_sanitises_snaps_and_always_publishes`, `WRITES`
- test `crates/params/tests/watch.rs`: `WRITES`

### P8. The restore and learned write: real units, silent

[Record section](transfer-from-seam-juce.md#p8-the-restore-and-learned-write-real-units-silent)

- `crates/host/src/state.rs`: `SavedState::load`
- `crates/learn/src/audio.rs`: `LearnAudio`
- `crates/learn/src/curve.rs`: `Curve::apply`
- `crates/params/src/decl.rs`: `ParamDecl::restore_value`, `ParamDecl::learned_value`
- `crates/params/src/program.rs`: `ProgramParams`, `Load::restore`
- `crates/params/src/set.rs`: module, `AudioParams::learned_write`
- test `crates/params/tests/alloc_gate.rs`: module
- test `crates/params/tests/set.rs`: module, `a_load_marks_the_restore_stored_before_writing_real_units`

### P9. One loud write path

[Record section](transfer-from-seam-juce.md#p9-one-loud-write-path)

- `crates/params/src/host_events.rs`: module
- `crates/params/src/program.rs`: `ProgramParams::sync`
- `crates/params/src/set.rs`: `ParamSet::store_then_publish`
- `crates/params/src/ui.rs`: module, `UiState::write`, `UiParams`
- test `crates/params/src/host_events.rs`: `loom_a_drain_racing_a_gesture_hears_one_balanced_bracket`
- test `crates/params/src/set/tests.rs`: `loom_a_user_write_and_a_sync_publish_their_count_and_request_after_their_value`
- test `crates/params/tests/writes.rs`: module

### P10. Identity and order

[Record section](transfer-from-seam-juce.md#p10-identity-and-order)

- `crates/host/src/cabi/abi.rs`: `SeamPlugin::param_info`
- `crates/host/src/cabi/mod.rs`: module
- `crates/host/src/state.rs`: `SavedState::save`
- `crates/params/src/decl.rs`: `Range::new`
- `crates/params/src/set.rs`: module, `ParamSetBuilder::build`, `declaration_order`
- test `crates/host/src/cabi/tests.rs`: module
- test `crates/host/tests/state.rs`: `set`, `a_block_is_version_parameters_extra_in_that_order_with_each_value_typed_by_kind`
- test `crates/params/tests/set.rs`: module
- test `examples/eq/tests/eq.rs`: `every_declaration_is_accepted_with_its_own_host_id`

### P11. Gestures are counted, and collapse

[Record section](transfer-from-seam-juce.md#p11-gestures-are-counted-and-collapse)

- `crates/params/src/host_events.rs`: module, `HostEventDrain`, `Requeue`
- `crates/params/src/ui.rs`: module
- test `crates/params/tests/writes.rs`: module, `a_panicking_emit_keeps_a_gestures_end_for_the_next_drain`

### P12. An open gesture holds back host-shaped writes from surfaces

[Record section](transfer-from-seam-juce.md#p12-an-open-gesture-holds-back-host-shaped-writes-from-surfaces)

- `crates/learn/src/target.rs`: `Target::held`
- `crates/params/src/lib.rs`: module
- `crates/params/src/ui.rs`: `ParamGesture::shown`, `UiParams::shown`, `Edit`
- `crates/params/src/watch.rs`: module, `Watch`
- test `crates/params/tests/watch.rs`: module, `a_watch_made_mid_drag_shows_what_the_others_show`, `a_second_token_opened_mid_drag_does_not_show_a_held_host_write`, `an_open_program_edit_holds_back_a_host_write`, `Model`, `every_watch_shows_what_the_model_says`
- test `crates/params/tests/writes.rs`: module, `a_touch_reaches_the_host_before_anything_changes`

### P13. How a change reaches a surface

[Record section](transfer-from-seam-juce.md#p13-how-a-change-reaches-a-surface)

- `crates/params/src/watch.rs`: `Watch`
- test `crates/params/tests/watch.rs`: `WRITES`, `poll_checked`

### P14. Subscribe, then seed

[Record section](transfer-from-seam-juce.md#p14-subscribe-then-seed)

- `crates/params/src/set.rs`: `ParamSet::read`
- `crates/params/src/watch.rs`: `Watch::new`
- test `crates/params/src/set/tests.rs`: `loom_a_read_never_has_a_count_newer_than_its_value`
- test `crates/params/tests/set.rs`: `WRITES`
- test `crates/params/tests/watch.rs`: `WRITES`, `watches_made_while_a_host_thread_writes_end_on_the_store`

### P15. Every surface keeps its own position

[Record section](transfer-from-seam-juce.md#p15-every-surface-keeps-its-own-position)

- `crates/params/src/watch.rs`: `Watch`
- test `crates/params/tests/watch.rs`: `two_watches_on_one_parameter_one_dropped_the_survivor_follows`, `every_watch_shows_what_the_model_says`

### P16. A surface shows the store, not the announcement

[Record section](transfer-from-seam-juce.md#p16-a-surface-shows-the-store-not-the-announcement)

- `crates/params/src/lib.rs`: module
- `crates/params/src/watch.rs`: module, `Watch`
- test `crates/params/tests/watch.rs`: module, `two_writers_racing_leave_the_watch_on_the_store`, `every_watch_shows_what_the_model_says`

### P17. Host-facing metadata

[Record section](transfer-from-seam-juce.md#p17-host-facing-metadata)

- `crates/host/src/cabi/mod.rs`: `param_info`
- `crates/params/src/lib.rs`: module
- `crates/params/src/meta.rs`: module, `stepped_decimals`
- `crates/params/src/set.rs`: `check_labels`, `DeclError::EmptyChoiceLabel`, `DeclError::PaddedChoiceLabel`, `DeclError::DuplicateChoiceLabel`
- test `crates/params/tests/meta.rs`: module, `text_uses_the_steps_decimals`, `text_counts_the_starts_and_the_steps_places_so_every_point_shows_as_itself`
- test `crates/params/tests/set.rs`: `bad_declarations_are_refused_in_every_build`

### P18. Coalesced and debounced observation

[Record section](transfer-from-seam-juce.md#p18-coalesced-and-debounced-observation)

- `crates/params/src/lib.rs`: module

## Undo

### U1. Enrollment by kind

[Record section](transfer-from-seam-juce.md#u1-enrollment-by-kind)

- `crates/undo/src/enroll.rs`: module, `History::enroll_param`, `History::enroll`
- `crates/undo/src/lib.rs`: module
- `crates/undo/src/list.rs`: `History::enroll_list`
- test `crates/undo/tests/enroll.rs`: module, `a_value_is_enrolled_once_in_every_build`
- test `crates/undo/tests/list.rs`: `a_forgotten_list_records_nothing_more_and_its_steps_replay_as_nothing`

### U2. Enrollment records nothing

[Record section](transfer-from-seam-juce.md#u2-enrollment-records-nothing)

- `crates/undo/src/enroll.rs`: `History::enroll_param`, `History::enroll`
- `crates/undo/src/list.rs`: `History::enroll_list`
- test `crates/undo/tests/enroll.rs`: `enrolling_records_nothing_and_earlier_writes_never_record`

### U3. Enrolling during an open gesture

[Record section](transfer-from-seam-juce.md#u3-enrolling-during-an-open-gesture)

- `crates/undo/src/enroll.rs`: `History::enroll_param`
- test `crates/undo/tests/enroll.rs`: `a_gesture_open_at_enrollment_records_from_its_open_value`

### U4. A gesture is one step

[Record section](transfer-from-seam-juce.md#u4-a-gesture-is-one-step)

- `crates/undo/src/enroll.rs`: `History::enroll_param`
- test `crates/undo/tests/enroll.rs`: `a_gesture_is_one_step_and_a_host_write_during_it_never_joins`, `a_plain_value_gesture_back_to_its_start_records_nothing_and_still_moves_the_baseline`

### U5. Writes outside a gesture

[Record section](transfer-from-seam-juce.md#u5-writes-outside-a-gesture)

- `crates/params/src/value.rs`: module
- `crates/undo/src/enroll.rs`: module, `Enrolled::baseline`, `Enrolled::flush`, `History::enroll`
- `crates/undo/src/lib.rs`: module
- test `crates/undo/tests/enroll.rs`: module, `outside_a_gesture_a_value_records_each_change_and_a_parameter_nothing`, `a_plain_value_gesture_records_from_its_first_open_after_what_came_before`

### U6. Transactions

[Record section](transfer-from-seam-juce.md#u6-transactions)

- `crates/undo/src/history.rs`: `Scope`, `History::transaction`
- `crates/undo/src/lib.rs`: module
- test `crates/undo/tests/restore.rs`: `a_transaction_open_across_a_clear_or_a_restore_starts_a_new_entry`
- test `crates/undo/tests/transaction.rs`: module, `every_step_in_a_transaction_joins_one_entry_named_by_the_outermost`, `an_undo_inside_a_transaction_ends_its_entry_and_the_next_keeps_its_name`

### U7. No-op writes record nothing, by a reflexive equality

[Record section](transfer-from-seam-juce.md#u7-no-op-writes-record-nothing-by-a-reflexive-equality)

- `crates/undo/src/enroll.rs`: `Enrolled::flush`
- `crates/undo/src/eq.rs`: module, `UndoEq`, `undo_by_eq`, `undo_by_fields`
- `crates/undo/src/history.rs`: module
- `crates/undo/src/lib.rs`: module
- test `crates/undo/tests/enroll.rs`: module, `a_no_op_write_keeps_the_redo_nan_payloads_included`, `a_plain_value_gesture_back_to_its_start_records_nothing_and_still_moves_the_baseline`

### U8. The baseline follows replay

[Record section](transfer-from-seam-juce.md#u8-the-baseline-follows-replay)

- `crates/undo/src/enroll.rs`: `Enrolled::baseline`, `Enrolled::poll`
- test `crates/undo/tests/enroll.rs`: `the_next_change_after_a_replay_starts_from_what_the_replay_left`
- test `crates/undo/tests/replay.rs`: module, `an_edit_after_undo_starts_from_the_undone_state`

### U9. A replay never records itself

[Record section](transfer-from-seam-juce.md#u9-a-replay-never-records-itself)

- `crates/params/src/list.rs`: `TrackedList`
- `crates/params/src/ui.rs`: `UiParams::replay`
- `crates/params/src/value.rs`: `TrackedRef::replay`
- `crates/undo/src/enroll.rs`: `ParamStep`
- `crates/undo/src/history.rs`: `Step`
- test `crates/undo/tests/enroll.rs`: `the_next_change_after_a_replay_starts_from_what_the_replay_left`
- test `crates/undo/tests/replay.rs`: module, `a_replay_drives_the_value_through_its_normal_path_and_records_nothing`

### U10. Writes caused by a replay

[Record section](transfer-from-seam-juce.md#u10-writes-caused-by-a-replay)

- `crates/params/src/lib.rs`: module
- `crates/params/src/list.rs`: `List`, `List::read`, `List::derived`
- `crates/params/src/set.rs`: `ParamSet`, `ParamSet::read`
- `crates/params/src/value.rs`: `Value`, `Value::read`, `Value::set_from`
- `crates/undo/src/enroll.rs`: `Enrolled::poll`
- `crates/undo/src/lib.rs`: module
- test `crates/params/tests/read.rs`: `a_value_derived_from_a_reading_passes_on_the_origin_of_the_value_it_used`
- test `crates/params/tests/writes.rs`: `every_write_leaves_its_origin_beside_the_change_count`
- test `crates/undo/tests/enroll.rs`: module, `a_write_derived_from_a_change_records_nothing_and_follows_its_replays`, `a_change_derived_from_a_parameter_or_a_list_records_nothing`, `a_list_kept_in_step_with_a_value_records_nothing_and_follows_its_replays`, `a_value_and_a_list_start_with_the_restore_origin`
- test `crates/undo/tests/restore.rs`: `a_write_under_the_applying_mark_is_the_restores_and_nothing_derived_records`

### U11. What a replay does

[Record section](transfer-from-seam-juce.md#u11-what-a-replay-does)

- `crates/params/src/ui.rs`: `UiParams::replay`
- `crates/undo/src/enroll.rs`: `ParamStep`
- test `crates/undo/tests/replay.rs`: module, `a_parameter_replay_is_one_complete_host_gesture_and_a_no_op_is_nothing`, `a_replay_drives_the_value_through_its_normal_path_and_records_nothing`

### U12. Replays are atomic (new in Rust)

[Record section](transfer-from-seam-juce.md#u12-replays-are-atomic-new-in-rust)

- `crates/undo/src/history.rs`: module, `History`, `History::record`
- test `crates/undo/src/history/tests.rs`: `a_request_made_during_a_replay_is_refused_and_the_cursor_moves_once`

### U13. Stale steps replay as no-ops

[Record section](transfer-from-seam-juce.md#u13-stale-steps-replay-as-no-ops)

- `crates/params/src/list.rs`: `TrackedList`
- `crates/params/src/value.rs`: `Shared::generation`, `Tracker::downgrade`
- `crates/undo/src/enroll.rs`: module, `ValueStep`, `History::forget_param`, `History::forget`
- `crates/undo/src/lib.rs`: module
- `crates/undo/src/list.rs`: module, `History::forget_list`
- test `crates/undo/tests/enroll.rs`: module, `a_forgotten_or_dropped_value_replays_as_nothing`
- test `crates/undo/tests/list.rs`: module, `every_element_drops_exactly_once_whatever_drops_first`, `a_forgotten_list_records_nothing_more_and_its_steps_replay_as_nothing`, `every_guard_on_a_steps_handle_makes_it_a_no_op`
- test `crates/undo/tests/replay.rs`: module

### U14. One history thread

[Record section](transfer-from-seam-juce.md#u14-one-history-thread)

- `crates/params/src/value.rs`: module
- `crates/undo/src/enroll.rs`: `Enrolled::flush`, `History::enroll`
- `crates/undo/src/history.rs`: module, `History`
- `crates/undo/src/lib.rs`: module
- test `crates/undo/tests/enroll.rs`: `writes_between_two_ticks_coalesce_latest_winning`

### U15. Host exclusion

[Record section](transfer-from-seam-juce.md#u15-host-exclusion)

- `crates/params/src/value.rs`: `Value`, `Value::write`
- `crates/undo/src/enroll.rs`: module, `History::enroll_param`
- test `crates/undo/tests/enroll.rs`: module, `a_gesture_is_one_step_and_a_host_write_during_it_never_joins`, `outside_a_gesture_a_value_records_each_change_and_a_parameter_nothing`, `only_a_plain_values_own_writes_record`, `a_derived_write_inside_a_drag_follows_its_origin`

### U16. Restores are not edits

[Record section](transfer-from-seam-juce.md#u16-restores-are-not-edits)

- `crates/params/src/clock.rs`: module, `RestoreWriter::applying`, `RestoreReader::applying`
- `crates/params/src/value.rs`: module, `Commit`, `Value`, `Value::write`
- `crates/session/src/lib.rs`: module
- `crates/session/src/ui.rs`: `UiState::apply`
- `crates/undo/src/enroll.rs`: `Enrolled::poll`
- `crates/undo/src/lib.rs`: module
- test `crates/params/tests/clock.rs`: `the_applying_mark_belongs_to_the_thread_that_took_it`
- test `crates/params/tests/writes.rs`: `a_user_write_while_a_restore_applies_is_refused_in_debug`
- test `crates/undo/src/history/tests.rs`: `a_replay_stops_before_its_next_write_when_a_restore_is_stored`
- test `crates/undo/tests/list.rs`: `a_verb_made_while_a_restore_applies_records_nothing`
- test `crates/undo/tests/restore.rs`: module, `history_from_before_a_restore_is_cleared_and_never_replays`, `writes_under_the_applying_mark_record_nothing_and_clear_nothing`, `a_plain_value_gesture_open_across_a_restore_records_from_the_restored_value`, `a_parameter_drag_across_a_restore_keeps_its_before_and_the_restore_stands`, `a_plain_value_gesture_across_a_restore_moves_the_baseline_with_no_write_after`, `a_write_under_the_applying_mark_is_the_restores_and_nothing_derived_records`

### U17. Lists

[Record section](transfer-from-seam-juce.md#u17-lists)

- `crates/params/src/lib.rs`: module
- `crates/params/src/list.rs`: module, `TrackedList`
- `crates/undo/src/lib.rs`: module
- `crates/undo/src/list.rs`: module, `ClearStep`, `History::enroll_list`
- test `crates/undo/src/list.rs`: `a_clear_undone_into_a_list_that_is_not_empty_keeps_its_elements`
- test `crates/undo/tests/list.rs`: module, `a_reset_records_nothing_and_makes_earlier_steps_replay_as_nothing`, `every_element_drops_exactly_once_whatever_drops_first`, `every_guard_on_a_steps_handle_makes_it_a_no_op`
- test `crates/undo/tests/restore.rs`: `list_verbs_made_after_a_reset_an_apply_or_a_restore_undo_as_usual`

### U18. The history listing

[Record section](transfer-from-seam-juce.md#u18-the-history-listing)

- `crates/undo/src/history.rs`: module, `History`, `History::record`
- test `crates/undo/src/history/tests.rs`: `undo_and_redo_each_move_the_listing_counter`, `a_step_joining_an_entry_moves_the_counter_only_when_pruning_drops_one`
- test `crates/undo/tests/restore.rs`: `the_listing_counter_reads_the_clock`
- test `crates/undo/tests/transaction.rs`: module, `an_undo_inside_a_transaction_ends_its_entry_and_the_next_keeps_its_name`

### U19. A bounded history

[Record section](transfer-from-seam-juce.md#u19-a-bounded-history)

- `crates/undo/src/history.rs`: module, `booked`, `State::prune`
- `crates/undo/src/size.rs`: module, `UndoSize`
- test `crates/undo/src/history/tests.rs`: `a_step_joining_an_entry_moves_the_counter_only_when_pruning_drops_one`, `a_dropped_redo_gives_back_its_units`
- test `crates/undo/tests/bound.rs`: module
- test `crates/undo/tests/list.rs`: module, `a_clear_books_every_element_it_holds`

## Session

### S1. One section per owner

[Record section](transfer-from-seam-juce.md#s1-one-section-per-owner)

- `crates/session/src/session.rs`: module, `kept`, `Session::save`
- `crates/session/src/ui.rs`: `SessionUi::register`
- test `crates/session/tests/session.rs`: module, `sections_compose_under_extra_and_nothing_to_save_leaves_the_key_out`

### S2. The session does not own participants

[Record section](transfer-from-seam-juce.md#s2-the-session-does-not-own-participants)

- `crates/session/src/lib.rs`: module, `Participant`
- `crates/session/src/ui.rs`: module, `Entry::live`, `SessionUi::register`, `Registration`
- test `crates/session/tests/session.rs`: `a_registration_and_its_session_end_in_either_order`, `a_participant_ending_its_registration_in_its_own_apply_is_not_recaptured`

### S3. Threads

[Record section](transfer-from-seam-juce.md#s3-threads)

- `crates/session/src/session.rs`: module, `Session`, `Session::restore`
- `crates/session/src/ui.rs`: `UiState::apply`
- test `crates/session/tests/session.rs`: `a_restore_stores_every_section_at_once_and_applies_on_a_later_tick`

### S4. Save reads snapshots

[Record section](transfer-from-seam-juce.md#s4-save-reads-snapshots)

- `crates/host/src/state.rs`: `SavedState::save`
- `crates/learn/src/mappings/session.rs`: `Learn::save`
- `crates/learn/src/saved.rs`: `encode`
- `crates/session/src/session.rs`: `Session::save`, `accept`
- `crates/session/src/ui.rs`: module, `SessionUi::register`, `Registration::changed`
- test `crates/session/tests/depth.rs`: module
- test `crates/session/tests/session.rs`: `a_save_runs_no_participant_code_and_a_change_recaptures`

### S5. Store now, apply later

[Record section](transfer-from-seam-juce.md#s5-store-now-apply-later)

- `crates/session/src/session.rs`: module, `Session::restore`
- `crates/session/src/ui.rs`: `SessionUi::tick`
- test `crates/session/tests/session.rs`: `a_restore_stores_every_section_at_once_and_applies_on_a_later_tick`, `two_host_threads_restoring_at_once_settle_on_one_applied_once`

### S6. The apply pass

[Record section](transfer-from-seam-juce.md#s6-the-apply-pass)

- `crates/session/src/ui.rs`: module, `UiState::passing`, `SessionUi::tick`
- test `crates/session/tests/session.rs`: `the_pass_applies_in_registration_order_and_recaptures_each`, `passes_never_nest`, `the_session_writes_the_restore_clock`, `a_tick_inside_a_late_apply_does_nothing`, `a_pass_that_left_a_busy_participant_owed_is_not_completed`, `a_registration_made_while_unwinding_owes_no_pass`
- test `crates/undo/tests/enroll.rs`: `clear_records_what_changed_first_so_nothing_from_before_it_lists_after`

### S7. Complete assignment, contained failures

[Record section](transfer-from-seam-juce.md#s7-complete-assignment-contained-failures)

- `crates/session/src/session.rs`: module, `kept`, `Session::restore`
- `crates/session/src/ui.rs`: `UiState::apply`
- test `crates/session/tests/session.rs`: `a_missing_key_a_null_section_or_no_object_resets_to_defaults`, `a_failed_or_panicking_apply_keeps_its_state_and_raw_section_and_the_others_apply`
- test `crates/undo/tests/enroll.rs`: `a_refused_undo_records_nothing`

### S8. A pending restore outranks live edits

[Record section](transfer-from-seam-juce.md#s8-a-pending-restore-outranks-live-edits)

- `crates/session/src/session.rs`: `State::restores`
- `crates/session/src/ui.rs`: `UiState::capture`, `Registration::changed`
- test `crates/session/tests/session.rs`: `a_pending_section_is_the_saved_one_until_its_apply_even_over_a_change`, `a_restore_landing_during_a_capture_wins_over_it`
- test `crates/undo/tests/restore.rs`: `steps_recorded_between_a_store_and_its_apply_are_dropped_at_completion`

### S9. The stale-apply stamp

[Record section](transfer-from-seam-juce.md#s9-the-stale-apply-stamp)

- `crates/session/src/session.rs`: module, `Stamp`, `State::restores`
- `crates/session/src/ui.rs`: `UiState::apply`
- test `crates/session/tests/session.rs`: module, `a_restore_landing_during_an_apply_keeps_its_section_and_applies_after`, `a_restore_landing_during_a_late_apply_keeps_its_section_and_applies_after`

### S10. Late participants

[Record section](transfer-from-seam-juce.md#s10-late-participants)

- `crates/session/src/lib.rs`: `Participant::apply`
- `crates/session/src/session.rs`: `State::unclaimed`, `State::register`, `Session::restore`
- `crates/session/src/ui.rs`: module, `SessionUi::register`
- test `crates/session/tests/session.rs`: `a_participant_registering_after_a_restore_applies_its_section_then_captures_it`, `a_failed_late_apply_keeps_the_raw_section_and_the_registration_stands`, `a_participant_registering_while_a_pass_is_owed_applies_once_at_registration`, `a_participant_registering_inside_another_apply_applies_once_with_the_newest_section`

### S11. Kept sections

[Record section](transfer-from-seam-juce.md#s11-kept-sections)

- `crates/session/src/session.rs`: `State::unclaimed`, `State::end`, `Session::restore`
- `crates/session/src/ui.rs`: `SessionUi::register`, `Registration`
- test `crates/session/tests/session.rs`: `sections_no_participant_holds_are_saved_back_verbatim`, `ending_a_registration_keeps_its_section_for_every_save_and_the_next_registration`, `a_restore_while_a_key_is_unheld_replaces_or_drops_its_kept_section`, `a_last_capture_of_nothing_leaves_nothing_behind`, `a_registration_ending_before_its_pending_apply_keeps_the_restored_section`

### S12. What a restore does to undo

[Record section](transfer-from-seam-juce.md#s12-what-a-restore-does-to-undo)

- `crates/session/src/ui.rs`: `UiState::apply`, `SessionUi::tick`
- test `crates/session/tests/session.rs`: module, `the_session_writes_the_restore_clock`

### S13. Parameters first

[Record section](transfer-from-seam-juce.md#s13-parameters-first)

- `crates/host/src/state.rs`: module, `SavedState`, `SavedState::load`
- test `crates/host/tests/state.rs`: module, `a_block_that_is_not_recognised_changes_nothing_at_all`, `parameters_are_assigned_before_the_session_applies_extra`, `parameters_land_before_a_participant_applies_while_the_ui_thread_ticks`

## MIDI learn

### L1. Registering a target is not mapping it

[Record section](transfer-from-seam-juce.md#l1-registering-a-target-is-not-mapping-it)

- `crates/learn/src/audio.rs`: `LearnAudio::refresh_slots`
- `crates/learn/src/mappings.rs`: module, `Learn`, `Learn::register`
- `crates/learn/src/table.rs`: module, `Table`
- test `crates/learn/src/table.rs`: `the_lookup_finds_what_a_scan_finds`
- test `crates/learn/tests/dispatch.rs`: module, `only_mapped_targets_take_a_slot`, `one_source_drives_every_target_mapped_to_it_and_a_target_has_one`
- test `crates/learn/tests/targets.rs`: `a_registered_target_starts_unmapped_and_its_key_is_its_own`

### L2. What can be learned

[Record section](transfer-from-seam-juce.md#l2-what-can-be-learned)

- `crates/learn/src/capture.rs`: `Policy`
- `crates/learn/src/mappings.rs`: `Config::reserved_nrpns`, `Config::reserved_ccs`, `Learn::map`, `policy`
- `crates/learn/src/parse.rs`: module
- `crates/learn/src/reservation.rs`: module, `CcReservation`, `NrpnReservation`, `ReservationError`
- `crates/learn/src/source.rs`: module, `Control::Cc`
- test `crates/learn/tests/capture.rs`: module, `a_refused_message_leaves_the_target_armed`, `a_reserved_controller_can_still_be_mapped_by_hand_and_dispatches`, `a_midi_2_assignable_controller_learns_as_an_nrpn`, `a_controller_reservation_on_one_group_leaves_the_others_free`
- test `crates/learn/tests/dispatch.rs`: module, `an_nrpn_assembles_and_a_high_byte_alone_reads_as_midi_1_says`, `a_partial_nrpn_sequence_is_dropped`, `rpn_data_entry_never_dispatches_as_the_last_nrpn`, `the_parser_runs_while_nothing_is_mapped`, `the_parser_owned_controllers_never_bind`, `each_group_and_channel_keeps_its_own_nrpn_state`, `reset_all_controllers_deselects_the_nrpn`, `a_reserved_nrpn_is_never_bound`, `the_null_parameter_number_selects_nothing`, `a_data_entry_low_byte_sent_first_waits_for_its_high_byte`, `a_new_address_drops_a_waiting_low_byte`, `a_new_address_high_byte_forgets_the_old_low_byte`, `reset_all_controllers_forgets_the_address`
- test `crates/learn/src/parse.rs`: `the_parser_agrees_with_a_model_of_midi_1_0`

### L3. The learn flow

[Record section](transfer-from-seam-juce.md#l3-the-learn-flow)

- `crates/learn/src/audio.rs`: `LearnAudio`
- `crates/learn/src/capture.rs`: module, `Policy`, `Stager::offer`
- `crates/learn/src/lib.rs`: module
- `crates/learn/src/mappings.rs`: `Learn`, `Learn::arm`, `Learn::map`, `Learn::edited`, `policy`
- test `crates/learn/tests/alloc_gate.rs`: `dispatch_neither_allocates_nor_frees`
- test `crates/learn/tests/capture.rs`: module, `arming_captures_the_next_message_and_the_poll_commits_it`, `several_captures_before_a_poll_commit_only_the_newest_legal_one`, `a_refused_message_leaves_the_target_armed`, `a_message_staged_for_one_arm_never_commits_to_the_next`, `cancelling_leaves_the_mappings_alone_and_drops_what_was_staged`, `editing_the_armed_targets_mapping_cancels_the_arm`, `an_nrpn_high_byte_waits_for_its_low_byte_through_polls_with_no_block_between`, `a_14_bit_controller_pair_learns_its_high_byte`, `a_controller_32_to_63_learns_itself_a_tick_later_unless_its_high_byte_came_just_before`, `an_arm_between_a_pairs_halves_waits_through_polls_and_learns_the_high_byte_that_follows`
- test `crates/learn/src/capture/tests.rs`: `loom_a_low_byte_in_the_next_block_is_never_missed`
- test `crates/learn/tests/saved.rs`: `a_restore_ends_the_arm_drops_old_held_entries_and_closes_bursts`
- test `crates/learn/tests/targets.rs`: `a_custom_target_learns_like_a_parameter_and_each_keeps_its_tier`

### L4. The dispatch snapshot

[Record section](transfer-from-seam-juce.md#l4-the-dispatch-snapshot)

- `crates/learn/src/snapshot.rs`: module
- `crates/learn/src/table.rs`: module
- test `crates/learn/tests/alloc_gate.rs`: module

### L5. The audio entry

[Record section](transfer-from-seam-juce.md#l5-the-audio-entry)

- `crates/learn/src/audio.rs`: module, `LearnAudio::process`
- test `crates/learn/tests/alloc_gate.rs`: module, `dispatch_neither_allocates_nor_frees`, `staging_ui_targets_and_polling_neither_allocate_nor_free`
- test `crates/learn/tests/dispatch.rs`: module, `messages_apply_in_order_so_the_block_ends_on_the_last`, `every_packet_reaches_the_processor_untouched_and_in_order`

### L6. From controller value to target value

[Record section](transfer-from-seam-juce.md#l6-from-controller-value-to-target-value)

- `crates/learn/src/curve.rs`: module
- `crates/learn/src/lib.rs`: module
- test `crates/learn/tests/dispatch.rs`: module, `a_curve_inverts_then_skews_then_windows`, `curves_are_checked_when_made`

### L7. Which origin a learned write uses

[Record section](transfer-from-seam-juce.md#l7-which-origin-a-learned-write-uses)

- `crates/learn/src/audio.rs`: module, `LearnAudio`
- `crates/learn/src/lib.rs`: module
- `crates/learn/src/mappings.rs`: `Custom`, `Learn`, `Learn::poll`, `Learn::drop`
- `crates/learn/src/table.rs`: module
- `crates/learn/src/target.rs`: module, `TargetId`, `Target`, `Target::held`, `BURST_IDLE_TICKS`
- test `crates/learn/tests/alloc_gate.rs`: `staging_ui_targets_and_polling_neither_allocate_nor_free`
- test `crates/learn/tests/saved.rs`: `a_restore_ends_the_arm_drops_old_held_entries_and_closes_bursts`
- test `crates/learn/tests/targets.rs`: module, `a_burst_is_one_gesture_of_the_newest_values_closed_after_15_idle_ticks`, `while_the_user_holds_the_target_the_burst_closes_and_the_value_waits`, `dropping_learn_closes_every_open_burst`, `a_custom_target_learns_like_a_parameter_and_each_keeps_its_tier`, `a_value_staged_while_the_user_holds_applies_once_they_let_go`, `each_new_value_restarts_the_idle_count`

### L8. Mapping changes take effect at once

[Record section](transfer-from-seam-juce.md#l8-mapping-changes-take-effect-at-once)

- `.github/workflows/ci.yml`: `loom:`
- `crates/learn/src/lib.rs`: module
- `crates/learn/src/mappings.rs`: module, `Custom::staging`, `Custom::close`, `Learn`, `Learn::detach`, `Learn::map`, `Learn::insert`, `Learn::unmap`
- `crates/learn/src/table.rs`: module, `Route::Staged`
- `crates/learn/src/target.rs`: module, `Staging`
- test `crates/learn/src/target.rs`: `loom_the_newest_staged_value_is_the_last_one_taken`
- test `crates/learn/tests/targets.rs`: module, `an_unmapped_target_closes_its_burst_once_and_a_new_mapping_starts_empty`, `a_detached_target_closes_its_burst_drops_and_is_unknown_after`, `a_re_pointed_target_never_applies_a_value_its_old_source_staged`

### L9. Saved mappings

[Record section](transfer-from-seam-juce.md#l9-saved-mappings)

- `crates/learn/src/lib.rs`: module
- `crates/learn/src/mappings.rs`: module, `DEFAULT_MAX_MAPPINGS`, `Config::max_mappings`, `Learn::held`, `Learn::register`, `Learn::detach`
- `crates/learn/src/mappings/session.rs`: module, `Learn::save`, `Learn::restore`, `Learn::attach`
- `crates/learn/src/saved.rs`: module
- test `crates/learn/tests/dispatch.rs`: `the_ceiling_refuses_a_new_mapping_but_never_a_re_point`
- test `crates/learn/tests/saved.rs`: module, `the_section_is_schema_1_sorted_by_key_with_the_identity_curve_left_out`, `save_restore_save_gives_the_same_section_and_routes_without_writing_values`, `a_restore_replaces_every_mapping_and_drops_a_bad_entry_on_its_own`, `an_entry_for_a_target_not_registered_yet_is_held_saved_back_and_lands_at_registration`, `attached_to_a_session_learn_saves_each_change_and_restores_through_it`, `a_restore_ends_the_arm_drops_old_held_entries_and_closes_bursts`, `a_held_entry_landing_or_dropping_at_registration_reaches_the_session`, `a_section_of_another_schema_is_refused_and_clears_the_routing`
- test `crates/learn/tests/targets.rs`: `a_detached_target_keeps_its_mapping_until_its_key_registers_again`

### L10. Learn in the menu and the poll

[Record section](transfer-from-seam-juce.md#l10-learn-in-the-menu-and-the-poll)

- `crates/learn/src/snapshot.rs`: module
- test `crates/learn/tests/alloc_gate.rs`: `staging_ui_targets_and_polling_neither_allocate_nor_free`

### L11. A learning plugin declares MIDI input

[Record section](transfer-from-seam-juce.md#l11-a-learning-plugin-declares-midi-input)

- `crates/host/src/lib.rs`: module

## The saved block and the host boundary

### H1. The saved block

[Record section](transfer-from-seam-juce.md#h1-the-saved-block)

- `Cargo.toml`: `float_roundtrip`
- `crates/host/src/lib.rs`: module
- `crates/host/src/plugin.rs`: `Plugin::saved`
- `crates/host/src/state.rs`: module, `STATE_VERSION`, `SEAM_VERSION`, `PLUGIN_VERSION`, `SavedState::save`, `SavedState::with_migration`
- `crates/host/src/vst3/mod.rs`: module
- test `crates/host/tests/state.rs`: module, `a_block_is_version_parameters_extra_in_that_order_with_each_value_typed_by_kind`, `save_load_save_gives_the_same_bytes_on_linear_skewed_and_stepped_ranges`, `a_save_racing_loads_writes_one_block_or_the_other_never_a_mix`, `save_load_save_is_a_fixpoint_for_any_double_in_a_section_or_a_kept_key`
- test `crates/session/tests/float_fixpoint.rs`: module
- test `examples/eq/tests/eq.rs`: `a_saved_state_loads_into_a_fresh_instance_on_another_thread`

### H2. Loading tolerates anything

[Record section](transfer-from-seam-juce.md#h2-loading-tolerates-anything)

- `adapters/juce/SeamPluginProcessor.cpp`: `PluginProcessor::setStateInformation`
- `crates/host/src/cabi/abi.rs`: `SeamPlugin::load`
- `crates/host/src/cabi/mod.rs`: `load`
- `crates/host/src/plugin.rs`: `Plugin::saved`
- `crates/host/src/state.rs`: `SavedState::load`, `versions`, `whole`
- `crates/host/src/vst3/component.rs`: `Component::setState`
- `crates/host/src/vst3/stream.rs`: `read_all`
- test `crates/host/src/engine/tests.rs`: `a_block_padded_with_nuls_loads_and_one_followed_by_anything_else_fails`
- test `crates/host/tests/hostile.rs`: module, `structured_blocks_keep_every_promise`, `random_bytes_never_panic_and_keep_every_promise`, `cut_flipped_and_padded_blocks_keep_every_promise`
- test `crates/host/tests/state.rs`: `a_block_that_is_not_recognised_changes_nothing_at_all`, `a_recognised_block_assigns_every_parameter_whatever_was_there_before`, `a_save_racing_loads_writes_one_block_or_the_other_never_a_mix`, `a_block_past_the_limit_is_refused_unread`, `a_number_past_f64s_range_is_refused_alone_and_the_rest_loads`, `a_version_past_u64s_range_or_written_as_a_float_still_loads`, `json_whitespace_around_the_object_is_part_of_the_block`
- test `examples/eq/tests/eq.rs`: `a_saved_state_loads_into_a_fresh_instance_on_another_thread`

### H3. Restore ordering

[Record section](transfer-from-seam-juce.md#h3-restore-ordering)

- `crates/host/src/lib.rs`: module
- `crates/host/src/state.rs`: module, `SavedState`, `SavedState::load`
- `crates/host/src/vst3/mod.rs`: module
- `crates/params/src/program.rs`: `ProgramParams::load`
- test `crates/host/tests/state.rs`: module, `parameters_are_assigned_before_the_session_applies_extra`, `a_load_marks_the_clock_every_history_of_the_set_reads`
- test `crates/params/tests/set.rs`: `a_load_marks_the_restore_stored_before_writing_real_units`
- test `crates/undo/tests/restore.rs`: `an_undo_between_a_loads_writes_and_its_store_replays_nothing`
- test `examples/eq/tests/eq.rs`: `loads_racing_the_audio_thread_end_on_the_last_one`

### H4. The learn host relay

[Record section](transfer-from-seam-juce.md#h4-the-learn-host-relay)

- no code site

### H5. Smaller boundary rules

[Record section](transfer-from-seam-juce.md#h5-smaller-boundary-rules)

- `adapters/juce/SeamPluginProcessor.h`: `PluginProcessor`
- `crates/host/src/plugin.rs`: `Plugin`
- `crates/params/src/set.rs`: `ParamSet::id_at`

## The page

### B1. The page seeds after subscribing

[Record section](transfer-from-seam-juce.md#b1-the-page-seeds-after-subscribing)

- `crates/bridge/src/protocol.rs`: `PageMessage::Ready`, `NativeMessage::Snapshot`, `NativeMessage::Reject`
- `page/src/client.ts`: `createClient`
- test `page/test/client.test.ts`: `sends ready only after the listener is installed`, `repeats until a snapshot arrives`

### B2. A drag owns the control

[Record section](transfer-from-seam-juce.md#b2-a-drag-owns-the-control)

- no code site

### B3. The page is typed from the declarations

[Record section](transfer-from-seam-juce.md#b3-the-page-is-typed-from-the-declarations)

- `crates/bridge/src/typescript.rs`: `param_map`, `protocol`
- `page/src/hooks.ts`: `createHooks`

### B4. Decoding never guesses

[Record section](transfer-from-seam-juce.md#b4-decoding-never-guesses)

- `crates/bridge/src/protocol.rs`: `PageMessage::decode`
- test `crates/bridge/src/protocol.rs`: `a_fractional_or_out_of_range_integer_is_rejected_not_truncated`, `unknown_fields_and_types_are_rejected`, `a_position_outside_0_to_1_is_refused_not_clamped`

### B5. A `ready` starts a new page session

[Record section](transfer-from-seam-juce.md#b5-a-ready-starts-a-new-page-session)

- no code site

### M1. Menus: A menu is flat, renderer-neutral data: items with namespaced string ids and parent-id nesting, in contribution order, invoked by id, never by position.

[Record section](transfer-from-seam-juce.md#m1-to-m3-menus)

- no code site

### M2. Menus: Contributors are asked in registration order and append items only for targets they recognise.

[Record section](transfer-from-seam-juce.md#m1-to-m3-menus)

- test `crates/params/tests/snap.rs`: `the_step_count_is_the_number_of_values_snapping_reaches`

### M3. Menus: Reset to default is a user edit inside one gesture (one host begin and end, one undo step), offered disabled when the value is already at its default.

[Record section](transfer-from-seam-juce.md#m1-to-m3-menus)

- no code site

## Properties the tests check

### property 1. Snapping is idempotent: a value snapped twice equals the value snapped once, for every kind.

[Record section](transfer-from-seam-juce.md#7-properties-the-rust-tests-check)

- test `crates/params/tests/snap.rs`: module, `snapping_twice_equals_snapping_once`, `snapping_inside_a_stepped_range_is_idempotent`

### property 2. One gesture: any writes inside one gesture give at most one step; it exists if and only if the final value differs from the value at open; undo gives the open value.

[Record section](transfer-from-seam-juce.md#7-properties-the-rust-tests-check)

- test `crates/params/tests/writes.rs`: `a_drag_back_to_its_start_makes_no_step`
- test `crates/undo/tests/enroll.rs`: module, `each_step_is_one_user_gesture_and_the_host_never_writes_one`, `a_plain_value_records_what_the_model_says`

### property 3. Transactions: one entry per outermost transaction that recorded anything; undoing it restores every touched value.

[Record section](transfer-from-seam-juce.md#7-properties-the-rust-tests-check)

- test `crates/undo/tests/transaction.rs`: module, `one_entry_per_outermost_transaction`

### property 4. Replay is pure: any sequence of undo, redo and jumps leaves the entry count unchanged, and the state after k applied steps is the same however k was reached.

[Record section](transfer-from-seam-juce.md#7-properties-the-rust-tests-check)

- test `crates/undo/tests/replay.rs`: module, `replay_is_pure`

### property 5. Edit after undo: the new step's before-value is the value after the undo.

[Record section](transfer-from-seam-juce.md#7-properties-the-rust-tests-check)

- test `crates/undo/tests/replay.rs`: `an_edit_after_undo_starts_from_the_undone_state`

### property 6. Host exclusion: with random host writes, program writes and gestures interleaved, the step count is the number of gestures with a net user change, and no host-written value is ever an after-value.

[Record section](transfer-from-seam-juce.md#7-properties-the-rust-tests-check)

- test `crates/undo/tests/enroll.rs`: `each_step_is_one_user_gesture_and_the_host_never_writes_one`

### property 7. No-op writes keep redo, NaN payloads included.

[Record section](transfer-from-seam-juce.md#7-properties-the-rust-tests-check)

- test `crates/undo/tests/enroll.rs`: `a_no_op_write_keeps_the_redo_nan_payloads_included`

### property 8. Host gesture balance: any interleaving of begins and ends on several controls reaches the host as balanced, non-overlapping begin and end pairs per parameter.

[Record section](transfer-from-seam-juce.md#7-properties-the-rust-tests-check)

- `crates/params/src/host_events.rs`: `HostEventDrain`
- test `crates/params/src/host_events.rs`: `loom_a_drain_racing_a_gesture_hears_one_balanced_bracket`
- test `crates/params/tests/writes.rs`: module, `check_balanced`, `the_host_sees_balanced_brackets`

### property 9. Two surfaces, one dropped: the survivor follows every later host write (P15).

[Record section](transfer-from-seam-juce.md#7-properties-the-rust-tests-check)

- test `crates/params/tests/watch.rs`: module, `every_watch_shows_what_the_model_says`

### property 10. Session: random restores, registrations and releases racing saves never lose a section, never save an older section after a newer one, and the last restore wins.

[Record section](transfer-from-seam-juce.md#7-properties-the-rust-tests-check)

- test `crates/session/tests/race.rs`: module

## Cited by the record, defined outside it

### C2. Edits reach the host right after each tick (cited in the VST3 exporter's decision).

Cited in the record's [D59
section](transfer-from-seam-juce.md#d59-the-shape-of-the-vst3-exporter).

- `adapters/juce/SeamPluginProcessor.h`: module
- `crates/host/src/cabi/mod.rs`: module
- `crates/host/src/engine/mod.rs`: `Engine::tick`, `Engine::tell_host`
- `crates/host/src/plugin.rs`: `Plugin::idle`
- `crates/host/src/vst3/mod.rs`: module

### C4. A sync through JUCE reaches the host as edits outside a gesture, which a host in latch mode may record; to check with AU in Logic and Pro Tools (cited in the JUCE adapter's decision).

Cited in the record's [D65
section](transfer-from-seam-juce.md#d65-what-a-host-sees-through-juce).

- `adapters/juce/SeamPluginProcessor.cpp`: `PluginProcessor::onRestart`

### C5. MIDI crosses the C interface as UMP both ways, with the exporters' MIDI input (cited in the C interface's decision).

Cited in the record's [D62 section](transfer-from-seam-juce.md#d62-the-c-interface-for-the-juce-adapter).

- `crates/host/src/cabi/mod.rs`: module
