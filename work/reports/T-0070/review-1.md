REVIEW T-0070 (round 1)
verdict: APPROVE
checks (run by reviewer): fmt pass (exit 0); clippy `--workspace --all-targets -D warnings` pass (exit 0); `cargo build --workspace` pass (exit 0); tests: nextest 871 run, 871 passed, 7 skipped (exit 0), with the lead's signal_store.rs patch applied; deny pass ("advisories ok, bans ok, licenses ok, sources ok"). All of these ran in one `git archive` scratch copy of c21d5f4 with its own CARGO_TARGET_DIR (CARGO_INCREMENTAL=0 and dev debuginfo 0 to save disk; test behaviour is the same). The copy and its target dir are deleted and nothing of mine is still running. Disk is tighter than the brief said: D: had 8.8 GB free, C: has 12.4 GB.

acceptance criteria:
- [x] A heard play's RateSession ends up in its play row, quantized; none with Speed off — verified by:
  - crates/tinwave-db/tests/derived_values.rs: a_rate_session_is_stored_into_the_play_row…, the_column_holds_hundredths_minus_fifty…, the_writer_stores_a_value_emitted_just_before_the_outcome_in_its_transaction (20 runs on the real writer thread), nothing_is_stored_with_the_speed_category_off / _tier_of_its_input_off, a_value_queued_before_the_category_was_switched_off_is_not_stored.
  - mapper.rs: the_value_is_quantized_to_hundredths, the_value_names_its_play_and_precedes_the_outcome_event.
  - controller_signals.rs: the_value_names_the_play_by_its_start_and_precedes_its_outcome, no_rate_session_with_the_speed_category_off.
  - All pass.
- [x] A Speed wipe clears it (signal_wipe rule table + test) — verified by:
  - `PLAY_COLUMNS` entry `rate` / `from: [RateSession]` in signal_wipe.rs, plus the `wipes()` rule (the kind or any of its `derived_inputs`).
  - Tests a_speed_wipe_clears_the_rate_and_keeps_the_play, a_wipe_of_what_the_rate_is_not_derived_from_keeps_it, a_derived_column_is_cleared_by_its_kind_and_by_every_input, a_derived_kind_is_wiped_with_the_tier_and_category_of_each_input (PerceivedLevel reached by T0, T1, Volume and OutputSound).
  - Mutation: setting `from: &[]` fails 2 tests.
  - The T-0073 statement-count pins hold: a_wipe_runs_a_fixed_number_of_statements…, a_context_wipe_runs_a_fixed_number…, a_batch_runs_a_statement_count… all pass. `rate` is folded into the existing play UPDATE, so the count is unchanged.
- [x] Storage estimate in catalog §9 and the growth test updated and passing — verified by:
  - All 8 storage_growth tests pass.
  - Rerun with `--no-capture`: "writer: 11471 transactions", play table 7191 B/day.
  - Baseline in the same copy with the sim's `emit_rate_session` call removed: also 11471 transactions, 7054 B/day. So the value rides the play-end transaction: 0 extra commits, about 1.1 B/play over a NULL column, about 2 B/play over no column. This matches §9 and storage-budget.md.
  - The same baseline mutation fails ninety_days_play_records_match_the_catalog_estimate and ninety_days_match_the_catalog_volume_table… ("10837 plays were simulated but 0 play rows hold their RateSession"), so the growth test guards the column.
- [x] fmt, clippy, nextest, deny pass — see checks. With the lead's patch, both signal_store tests pass. Without the patch line, schema_matches_database_md fails as described.

Lead focus items:
1. **Architecture and ADR 0010.**
   - A third bounded queue, merged by seq, fits architecture.md ("Playback → SignalBus → writer"). Dependency direction is unchanged (db → signals, playback → signals).
   - ADR 0010 is the next free number. No other branch adds an ADR.
   - Spec §10.1 says nothing about derived values or the bus shape, so no spec change is needed. Neither ADR 0001 nor 0002 is contradicted.
   - The wipe-in-flight gap is stated honestly in the ADR's Consequences. a_speed_wipe_clears_the_rate test even shows a post-wipe value repopulating a pre-wipe row, which is that gap (T-0082).
2. **Playback never blocks.**
   - `emit_derived` (bus.rs) does an atomic load, a fetch_add, `clock.now`, a cheap closure and `try_send` on a bounded channel. No lock, no wait.
   - Called from the controller thread (mapper), never from the render callback.
   - rt_alloc (4 tests), controller_rt_alloc and output_rt_alloc all pass.
3. **Consent.**
   - `required_bits` (settings.rs) needs the category and tier of the kind and of every input. That is consistent with privacy-and-signals.md ("at the source", and the wipe rule "cleared by every category it is derived from"), and stricter than before.
   - The catalog row, catalog header paragraph and privacy doc are updated.
   - Toggle-off is checked at emit (`emit_derived`), at drain (`take_derived`) and at commit (`try_commit` filters stage and carried values through `drain.enabled`).
   - The Signal Center has no signal list yet. Only a sidebar entry exists (qml/Sidebar.qml:137). Confirmed.
4. **Storage.**
   - Migration 0003 is a single nullable INTEGER and append-only. database.md's SQL block is in sync (the doc-vs-migration test passes).
   - The UPDATE hits indexed lookups (UNIQUE uuid on dict_*/session, `play_rec_ts`).
   - Hold-one-more-commit rule: values are re-tried once and then counted in `derived_unmatched`; at shutdown they are counted too.
5. **Wipes:** see criterion 2.
6. **Mutations** (each fails the expected tests, then restored and touched):
   - Builder's:
     - Drain consent replaced by `if true` fails a_value_queued_before_its_kind_was_switched_off….
     - Writer commit consent removed fails a_staged_value_whose_kind_was_switched_off… and a_held_value_whose_kind_was_switched_off….
     - `rate` not registered in `PLAY_COLUMNS` fails 2 tests (see criterion 2).
     - Unmatched value never held fails 5 tests (4 writer unit tests plus derived_values a_value_of_a_play_that_has_no_outcome…).
   - Mine:
     - Seq merge comparison inverted (`d > e`) fails a_batch_cut_never_puts_a_value_before_an_event_emitted_before_it.
     - Writer holds unmatched values forever (held filter = all) fails a_value_with_no_play_is_dropped_at_the_second_commit.

blocking findings:
- none

design check: n/a (no UI change)

non-blocking findings:
- crates/tinwave-signals/src/bus.rs:812-816 — the `fill` doc says "A producer sends the event that ends a play before it sends that play's derived value". The controller does the opposite (value just before the outcome). The invariant still holds for one producer thread (any lower-seq event is already sent), but the sentence is left over from the first "emit after" design and should be reworded.
- crates/tinwave-db/src/play.rs:235 — `UPDATE_PLAY_RATE` is one 220-character literal with runs of five spaces (a collapsed `\` continuation). It is unlike the other SQL constants. Cosmetic.
- crates/tinwave-signals/src/bus.rs, `SignalSink::write_with_derived` — the default silently discards derived values. Today only test sinks rely on it (SignalStore and Stage override it), but a future storing sink that forgets to override loses consented values without a count. Consider making it required, or counting the discard.
- docs/rules/storage-budget.md:67 ("~58 bytes" per play record) and the signal-catalog §9 "~6 KB" play-record total were not bumped. The text adds ~2 B and ~0.25 KB/day, but the totals stay; measured is 7.19 KB/day (ratio 1.20, within tolerance).
- The growth harness only prints the transaction count, it does not assert it. The "never a commit of its own" property is pinned by writer::tests::a_value_goes_into_the_transaction_of_the_outcome_that_follows_it (`batches_committed == 2`) and a_value_alone_does_not_make_the_stage_due…, not by the growth test.

proposals:
- P: Pin the growth run's transaction count, or assert that it equals a no-derived run | why: makes "a derived value never adds a commit" a budget-level guard, not just a unit-test one | size: S
- P: Add the derived-value UPDATE to the batch statement-count test (one statement per value, independent of stored rows) | why: the T-0073 batch pin only covers `write_batch` without derived values | size: S

notes for lead:
- At merge, apply the signal_store.rs patch after T-0075 lands. T-0075 also edits tests/signal_store.rs (43 lines), tests/signal_wipe.rs and tests/common/mod.rs. The patch file has CRLF line endings: `git apply` needed `--ignore-whitespace`, and I normalised the file to LF afterwards so fmt stays clean.
- The branch merges cleanly into the current main (6bbeee3; `git merge-tree` exit 0). Its files do not overlap T-0068 or T-0075.
- Out-of-scope files: each is needed (writer.rs, signal_wipe.rs, signals settings.rs, error.rs and test_support.rs, playback lib.rs re-export, db and playback tests, privacy and adding-a-signal docs, ADR). None is gratuitous. wiring.rs is untouched.
- PerceivedLevel has no stored form, so it could not be seeded in the wipe fixtures. The generic rule is unit-tested with its real inputs.
- Gap left for later (not from this ticket): a Speed off-then-on toggle within one play still yields a value. That was already the case under T-0062.
