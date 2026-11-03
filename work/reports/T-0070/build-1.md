HANDOFF T-0070 (round 1)
status: partial   (all work is done; the branch is red on exactly two tests in a file I was told not to edit. The 8-line fix is below and verified.)
branch: t/T-0070-derived-values-and-rate-session-storage   commit: c21d5f4
summary: Derived values are now a first-class path. tinwave-signals carries a `DerivedValue` on the bus (`SignalBus::emit_derived`, a third queue merged with the events by seq, `SignalSink::write_with_derived`). The writer stores `RateSession` as a nullable `u8` `play.rate` (migration 0003), in the same transaction as the play-end event. `take_rate_sessions`, the `RateSession` type and `rate_sessions_dropped` are removed. Consent follows the inputs and is re-checked three times. Wipes clear the column by the derived kind and by every input it is computed from.

acceptance criteria:
- [x] A heard play's RateSession ends up in its play row, quantized; none with Speed off. Proof, all in the new file crates/tinwave-db/tests/derived_values.rs unless noted:
  - a_rate_session_is_stored_into_the_play_row_of_its_play_after_the_outcome_was_committed
  - a_value_in_the_batch_of_its_outcome_finds_the_row_written_in_that_batch
  - the_column_holds_hundredths_minus_fifty_and_a_play_never_measured_holds_null
  - the_writer_stores_a_value_emitted_just_before_the_outcome_in_its_transaction (20 iterations on the real writer thread)
  - nothing_is_stored_with_the_speed_category_off, nothing_is_stored_with_the_tier_of_its_input_off, a_value_queued_before_the_category_was_switched_off_is_not_stored
  - controller side: mapper tests the_value_is_quantized_to_hundredths and nothing_is_derived_with_the_speed_category_off (src/controller/mapper.rs), and controller_signals.rs a_heard_play_has_one_rate_session_with_the_time_weighted_rate_of_its_schedule, the_value_names_the_play_by_its_start_and_precedes_its_outcome, no_rate_session_with_the_speed_category_off
- [x] A Speed wipe clears it (rule table plus test). Proof: a_speed_wipe_clears_the_rate_and_keeps_the_play (it also checks a new value is stored again after the wipe) and a_wipe_of_what_the_rate_is_not_derived_from_keeps_it (T1/T2/T3, Volume, Browsing, OutputSound and Session keep it). Rule table: `PLAY_COLUMNS` has `rate` with `from: [RateSession]`. The generic `wipes()` rule (derived kind plus its `derived_inputs`) is pinned by signal_wipe.rs unit tests a_derived_column_is_cleared_by_its_kind_and_by_every_input and a_derived_kind_is_wiped_with_the_tier_and_category_of_each_input.
- [x] Storage estimate in catalog §9 and the growth test updated and passing. The growth sim now emits a RateSession for every simulated play, and the harness asserts every play row holds one (`plays_with_rate`); `storage_growth.rs` compares `rate` in `play_rows`. All 8 storage_growth tests pass (workspace run). The 90-day run still takes 11 471 transactions, the same as without derived values, so the value does ride the play-end transaction. Play table 7191 B/day against 7054 B/day with `rate` NULL, so about 1 B more than a NULL and about 2 B (0.25 KB/day) over the table before the column. §9 and storage-budget.md say so.
- [ ] fmt, clippy, nextest, deny pass. Not fully: fmt pass, clippy -D warnings pass (`--workspace --all-targets`), deny pass, `cargo doc` with -D warnings pass. nextest: 871 run, 869 pass, 2 fail, 7 skipped (the ignored real-device tests). The 2 failures are tinwave-db::signal_store `migrations_apply_on_an_empty_database_and_are_idempotent` and `schema_matches_database_md`. They hard-code "2 migrations" and the 12 `play` columns, which any migration breaks. They are in signal_store.rs, one of the three files I was told not to touch (T-0075). The patch is below. With it applied, all 23 signal_store tests pass, including the statement-count test. I applied it only temporarily and reverted it with `git checkout`, so the file is untouched on the branch. The patch is also at C:\Users\FSYY\AppData\Local\Temp\claude\D--dev-Tinwave\53bf9027-c019-4dd4-82f1-5a72c6705b0c\scratchpad\T-0070-signal_store.patch.
- Other items in the ticket:
  - take_rate_sessions stopgap removed (Controller, ControllerHandle, RateSessionQueue file, RateSession type, rate_sessions_dropped). Done.
  - P-0278, consent re-checked at emit, at the drain and at the writer's commit. Tests: bus a_value_queued_before_its_kind_was_switched_off_is_dropped_not_delivered, writer a_staged_value_whose_kind_was_switched_off_is_withdrawn_at_the_commit and a_held_value_whose_kind_was_switched_off_is_withdrawn_too, mapper a_value_queued_before_speed_was_switched_off_is_not_delivered. The wipe half is NOT done, see deviations.
  - P-0296, rule implemented and tested (T0 wipe, Volume, OutputSound and T1 all reach a PerceivedLevel-style kind). PerceivedLevel itself has no value, column or producer, so there is no PerceivedLevel row to seed, see deviations.
  - ADR 0010 written.

checks: fmt pass, clippy pass, tests 869 passed / 2 failed (both fixed by the patch below), deny pass
files changed (30): 
- tinwave-signals/src: derived.rs (new), bus.rs, lib.rs, error.rs (`NotDerived`), settings.rs, test_support.rs
- tinwave-db: migrations/signals/0003_play_rate.sql (new), src/store.rs, play.rs, writer.rs, signal_wipe.rs; tests/derived_values.rs (new), tests/growth/sim.rs, tests/growth/mod.rs, tests/storage_growth.rs, tests/signal_settings.rs (migration count 2 to 3)
- tinwave-playback: src/controller/mapper.rs, mod.rs, runtime.rs, types.rs, rate_session.rs (deleted), src/lib.rs; tests/controller_signals.rs, tests/controller_support/mod.rs
- docs: adr/0010-derived-values-on-the-bus.md (new), guides/signal-catalog.md, guides/adding-a-signal.md, rules/database.md, rules/storage-budget.md, rules/privacy-and-signals.md
signals added/changed: no new kind. `RateSession` (derived, T0, Speed category) is now stored: `play.rate`, about 2 B per play, about 0.25 KB/day, 0 B in `signal_event`. Catalog row and §9 play-records row are updated. There is no Signal Center UI or code list yet, so nothing to update there. Behaviour change in `SignalSettings::enabled`: a derived kind now needs the category and tier of every input, not only its own tier (PerceivedLevel now also needs T0 and OutputSound; RateSession is unchanged). Tests updated, catalog and privacy rules updated.
ui: none
docs updated: signal-catalog.md, adding-a-signal.md, database.md (SQL block plus paragraph; the doc-vs-migration sync test passes), storage-budget.md, privacy-and-signals.md, ADR 0010
deviations from ticket:
- Files outside the listed scope, all unavoidable: writer.rs, signal_wipe.rs and lib.rs-adjacent code in tinwave-db/src (the writer must stage derived values, and the ticket itself says to register the column in signal_wipe.rs); tinwave-signals/src/settings.rs, error.rs and test_support.rs; the playback tests controller_signals.rs and controller_support/mod.rs (they used take_rate_sessions); tinwave-db tests growth/*, storage_growth.rs and signal_settings.rs; docs privacy-and-signals.md, adding-a-signal.md and the new ADR. Nothing under work/. wiring.rs was not needed: the app does not build a bus, writer or wiper yet.
- signal_store.rs not edited, per your parallel-safety note. I did not touch signal_wipe.rs (tests) or common/mod.rs. I copied no helpers from them; my own file has its own Rig. signal_settings.rs (not on your list) had the same migration-count assertion, so I changed `2` to `3` there.
- The wipe-in-flight half of P-0278 is not done. A value queued or staged (at most one flush interval) when a wipe commits is still stored afterwards, exactly like an event in that position. `SignalWiper` is a separate connection with no link to the bus or writer, and the app wires neither. Toggle-off is fully handled. This is written down in ADR 0010 under Consequences. See the proposal.
- PerceivedLevel row in the wipe fixtures: not possible, because PerceivedLevel has no stored form. What exists is the generic rule (`wipes()`) with unit tests that use PerceivedLevel's real inputs. The existing signal_wipe.rs fixtures could not be edited anyway.
- The value is emitted just before the outcome event, not after it. I first emitted it after and measured 22 077 transactions against 11 471 (every play committed twice). So the controller emits it before, the bus merges it with the events by seq, the writer does not commit a value alone, and a value that finds no row is held for exactly one more commit (then dropped and counted in `WriterStats::derived_unmatched`; a stopped play has no outcome, so it ends up there). Order-independent: a producer that emits it after still works, one flush interval later (tested).
- `SignalSink` gained `write_with_derived` with a default that forwards to `write` and discards derived values. That is deliberate for test collectors (documented). `SignalStore` and the writer's stage override it.
- Stopped plays: their RateSession is emitted and then dropped and counted by the writer, because there is no play row for a stop. This is the documented limitation in the catalog row and the controller docs.

Patch for tests/signal_store.rs (apply with `git apply`; verified: signal_store 23/23 pass with it):
```
@@ -78,16 +78,16 @@ fn migrations_apply_on_an_empty_database_and_are_idempotent() {
-    assert_eq!(count(&r, "SELECT count(*) FROM schema_migration"), 2);
+    assert_eq!(count(&r, "SELECT count(*) FROM schema_migration"), 3);
-    assert_eq!((report.applied, report.version), (0, 2));
+    assert_eq!((report.applied, report.version), (0, 3));
-    assert_eq!(count(&r, "SELECT count(*) FROM schema_migration"), 2);
-    assert_eq!(SIGNALS_MIGRATIONS.len(), 2);
+    assert_eq!(count(&r, "SELECT count(*) FROM schema_migration"), 3);
+    assert_eq!(SIGNALS_MIGRATIONS.len(), 3);
@@ -195,7 +195,8 @@ fn schema_matches_database_md() {
             "peak_section",
-            "context_key"
+            "context_key",
+            "rate"
```

Revert tests I ran (each mutation made the named tests fail, then the file was restored and touched):
- bus drain consent re-check replaced by `if true` fails a_value_queued_before_its_kind_was_switched_off_is_dropped_not_delivered.
- the merge taking a value before earlier events fails a_batch_cut_never_puts_a_value_before_an_event_emitted_before_it.
- settings ignoring derived inputs fails 2 settings tests.
- writer commit-time consent removed fails 2 writer tests.
- a lone value made due fails a_value_alone_does_not_make_the_stage_due_but_the_interval_does.
- unmatched value not held fails 4 writer tests.
- `wipes()` ignoring inputs fails 2 signal_wipe unit tests.
- `rate` column not registered for the wipe fails a_speed_wipe_clears_the_rate_and_keeps_the_play.
- UPDATE ignoring the session or the recording fails a_value_is_stored_only_into_the_play_it_names (the test first passed under the session mutation, so I strengthened it).
- values not applied in the batch fails 8+ derived_values tests.
- controller not emitting fails 8 mapper tests.
Also a build-freshness check: not run for this ticket (no build.rs touched).

proposals:
- P: Link SignalWiper to the bus so a wipe also drops in-flight values and events | why: closes the "wipe of already-queued data" gap (P-0278 second half); a wipe generation on `SignalSettings` that the wiper bumps and the drain/stage compare, wired where the app builds both | size: S | evidence: ADR 0010 Consequences; wipe.rs `SignalWiper::open(path)` has no handle to the bus
- P: Write a play record for stopped and quit plays | why: their RateSession is currently counted and dropped (`WriterStats::derived_unmatched`); the same gap blocks `EndReason::Stopped`/`Quit` | size: M | evidence: play.rs module docs "Stopped and quit plays cannot be derived yet"
- P: Add the `PerceivedLevel` producer, `DerivedPayload` variant and play column | why: consent, wipe and transport rules for it already exist; adding it is a variant, a nullable column, a `write_derived` branch and one `PLAY_COLUMNS` entry (steps in adding-a-signal.md) | size: M | evidence: derived.rs doc, play.rs `write_derived` match without wildcard
- P: Make the signal_store.rs migration-count asserts follow `SIGNALS_MIGRATIONS.len()` | why: every future migration breaks the same hard-coded 2/3 and the play column list | size: S | evidence: tests/signal_store.rs:81-90

questions for lead:
- Apply the signal_store.rs patch above at merge (or tell me to commit it in round 2 once T-0075 has merged).
- Is the wipe-in-flight gap acceptable for now (documented in ADR 0010), or should the first proposal above become part of this ticket?
- One thing I could not check: `git status` shows a foreign python process (pid 60496, `t.py cmd ... cmake-build.cmd configure`, started 2-11-2026 10:04). It is not mine and I left it alone. Nothing of mine is still running (no tinwave-desktop, cargo, nextest or python from this ticket).
