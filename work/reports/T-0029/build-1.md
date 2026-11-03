HANDOFF T-0029 (round 1)
status: partial
branch: t/T-0029-live-play-records   commit: 420dd6e
summary: Every newly stored Completed or Skipped now writes exactly one `play` row in the same transaction as its batch. The row is paired with its PlayStarted per device by seq (never per session), carries a ContextKey read from the start's snapshot, and a failed track writes no row. `partial` for one reason: Stopped and Quit plays cannot be derived, because the signal stream has no event that says a play was stopped or quit (details under questions). The growth test accounts for play rows and passes within its CPU budget.

acceptance criteria:
- [~] Each play end (Completed, Skipped*, Stopped, Quit) writes exactly one play row in the same transaction.
  - Done for Completed and SkippedEarly/Mid/Late. proof: `play_records::each_play_end_writes_exactly_one_row`, `the_play_row_is_written_in_the_transaction_of_its_outcome` (a trigger-failed `play` insert rolls back the batch's events; the retry writes the row once; a re-sent batch writes none), `the_writer_commits_the_play_row_with_the_play_end` (through bus and writer thread).
  - Not done: Stopped and Quit. The playback controller emits no signal for them (controller/mod.rs table: `TrackLeft{Stopped}` below 97% and at quit gives nothing), so they look exactly like a failure after the track was heard (a bare PlayStarted). `EndReason::Stopped` (4) and `Quit` (5) exist as frozen codes; nothing writes them.
- [x] A failed track writes no play row.
  - proof: `a_failed_track_writes_no_row`. There is no `TrackLeft` signal event; Failed produces no signal at all, so the failed play is a bare PlayStarted that no outcome closes. The next play pairs with its own start.
  - Also covered: `an_outcome_without_its_start_writes_no_row` (an unpaired outcome writes no row and is counted as `plays_unpaired`).
- [x] `context_key` is packed by a `ContextKey` type with a documented, versioned layout; round-trip and unknown values tested; a play with no captured context stores 0.
  - proof: tinwave-signals `context_key::tests` (8 tests: frozen v1 layout `0x1015_9B76` for a full key, each field in its own bits, every value of every field round-trips, unknown is 0, invalid values rejected).
  - proof: `play_records::the_context_key_comes_from_the_snapshot_of_the_start_and_defaults_to_unknown` (no ctx, a missing snapshot, an unreadable key and a negative key all store 0).
- [x] `peak_section` is NULL. proof: asserted in `each_play_end_writes_exactly_one_row`.
- [x] Growth test accounts for play rows.
  - proof: `ninety_days_play_records_match_the_catalog_estimate` (rows equal the simulated plays, none with key 0, 120.4 rows/day, 6 963 B/day against 6 000, ratio 1.16, limit 1.5).
  - proof: the extrapolation now uses the measured warm tier (2.5 MB/year) instead of the catalog's 2.0; 5 years with tiering is 19.6 MB against the 60 MB budget.
  - proof: `the_catalog_still_says_what_the_test_expects` also pins the "Play records" row of catalog section 9.
- [x] P-0111: start and outcome pair per device by seq. proof: `an_outcome_in_a_later_session_pairs_by_device_not_session` (start in session 1, Skipped in session 2, separate batches; row belongs to session 1; the 44 min pause is excluded from listened), `outcomes_pair_per_device`, `the_order_of_the_events_in_a_batch_does_not_matter`.
- [x] P-0127 tidy-ups.
  - `settings.rs` `the_mask_is_read_under_the_connection_lock` now uses a drop guard (`ReleaseOnDrop`) that releases the parked thread, so a failed assert fails instead of hanging. Checked by flipping an assert: the test fails in 0.04 s.
  - The trigger SQL literal in `tests/signal_settings.rs` is a proper `\` continuation.

checks: fmt pass, clippy pass, tests 644 passed / 0 failed (6 skipped by nextest config) for `cargo nextest run --workspace`, deny pass (advisories, bans, licenses, sources ok)

mutation checks (play_records, each reverted afterwards; each mutation killed by the test named):
- drop the device filter in the start lookup: killed by `outcomes_pair_per_device`
- drop the recording-equality check: killed by `an_outcome_without_its_start_writes_no_row`
- pair only within the latest session: killed by `an_outcome_in_a_later_session_pairs_by_device_not_session` and `outcomes_pair_per_device`
- drop the pause subtraction: killed by the pauses test and the later-session test
- skip ContextKey validation: killed by the context-key test
- use the outcome's snapshot as a fallback: killed by the context-key test
- derive a row for duplicate events: killed by the transaction/retry test

files changed:
- crates/tinwave-signals/src/context_key.rs (new)
- crates/tinwave-signals/src/lib.rs
- crates/tinwave-db/src/play.rs (new)
- crates/tinwave-db/src/store.rs
- crates/tinwave-db/src/writer.rs
- crates/tinwave-db/src/lib.rs
- crates/tinwave-db/src/settings.rs
- crates/tinwave-db/tests/play_records.rs (new)
- crates/tinwave-db/tests/storage_growth.rs
- crates/tinwave-db/tests/growth/mod.rs
- crates/tinwave-db/tests/growth/sim.rs
- crates/tinwave-db/tests/signal_settings.rs
- docs/guides/taste-engine.md
- docs/rules/storage-budget.md

signals added/changed: none (no new signal kind). The warm-tier "Play records ~120 rows, ~6 KB/day" row already exists in catalog section 9 and is now measured.

ui: none

docs updated:
- docs/guides/taste-engine.md: new "ContextKey layout (version 1)" section.
- docs/rules/storage-budget.md: the `context_key` line now points at that layout and says 0 means nothing known.

deviations from ticket:
1. **All-unknown key is raw 0, with no version bits.** The ticket fixes bits 28-31 as version 1 and also says "the all-unknown key (0)". Those conflict. I made 0 the only key without version bits: `raw()` writes version 1 only when at least one field is known, and `from_raw` accepts 0 and version 1. A version-1 value with every field 0 reads back as unknown. This is documented in the type and in taste-engine.md.
2. **ContextKey device class** is the existing `OutputClass` code plus one, and `OutputClass::Unknown` maps to 0, so all 8 values fit 3 bits. Weekday type uses `Weekday` (taste-engine.md wording), not "workday".
3. **No enum overflowed its field.** daypart 7/7 codes, weekday type 3/3, weather condition 6/7, temperature band 5/7, device class 7/7, activity 9/15, place slot 63/63. The daypart and weekday type use every code, so a new value of either needs a new layout version.
4. **ContextKey is built from new enums.** `Daypart`, `WeekdayType`, `WeatherCondition`, `TemperatureBand`, `ActivityGuess` and `PlaceSlot` are defined in tinwave-signals `context_key.rs`; none existed yet. The context engine ticket should reuse them.
5. **Context source.** The writer reads the `key` column of the `context_snapshot` row that the PlayStarted event's `ctx` points at, in the same transaction. That is reachable and not invented. In production nothing writes `context_snapshot` rows yet, so every real play stores 0 until the context engine writes snapshots with a valid ContextKey. The growth harness now inserts snapshot rows with valid keys before the event names them (previously it inserted them after the play, with random keys).
6. **Growth test edits.** `the_same_seed_gives_the_same_sizes` compared file sizes for "different seed is different data". That became a page-granularity coincidence once play rows were added, so it now compares stored event and play rows. The writer-CPU comment is updated: play records cost the writer about 15% more (debug 7.0-7.4 s, release 3.6 s; budgets 15 s and 8 s unchanged and still met).
7. **Flags and end reasons.**
   - `PlayFlags` bit order follows the storage-budget.md list (bit 0 favorited, 2 seek_back). Only FAVORITED and SEEK_BACK are derived, from `Favorited` and `SeekBack` events of the same recording during the play.
   - The other documented flags have no usable source today: replayed, seek_fwd_intro, vol_*, rate_*, lyrics, queued, removed_after. They stay 0.
   - `end_frac` for Completed is 255. Skipped uses its fraction.
   - The end-reason thresholds are mine: SkippedEarly is under 30 s listened (the catalog's two negative buckets), SkippedLate is past 80% (the catalog's positive bucket), otherwise Mid. They are in the `EndReason` docs (`EndReason::EARLY_BELOW_DS`, `LATE_ABOVE_RAW`).
   - Skipped `listened_ds` is wall time from start to skip minus paused time, derived from Paused/Resumed events (Skipped carries no listened time).

proposals:
- P: Emit a play-end signal for stops and quits | why: lets the writer produce `Stopped`/`Quit` rows and tell a stop from a decode failure after the track was heard (both are a bare PlayStarted today) | size: M | evidence: crates/tinwave-playback/src/controller/mod.rs (the `TrackLeft { Stopped }` row of the mapping table), docs/rules/database.md:81
- P: Document `end_reason` thresholds and `PlayFlags` bits in storage-budget.md and database.md | why: the thresholds and bit order are only in Rust docs (`EndReason`, `PlayFlags`) because the ticket scope limited me to the context_key line | size: S | evidence: crates/tinwave-db/src/play.rs
- P: Cover more `PlayFlags` once their sources exist (replayed, seek_fwd_intro after audio analysis, vol_*/rate_* by decoding payloads in the window scan) | why: flags are currently 2 of 11 | size: M | evidence: crates/tinwave-db/src/play.rs `scan_window`
- P: Context hub must write `context_snapshot.key` through `ContextKey::raw()` | why: otherwise play rows keep storing 0 (an invalid key is rejected with a warn and stored as 0) | size: S | evidence: crates/tinwave-db/src/play.rs `context_key`

questions for lead:
- Is "partial" acceptable for the Stopped/Quit gap, or should a follow-up ticket add the signal (see the first proposal) before T-0029 is accepted? The ticket text assumes `TrackLeft` events reach the hot tier, but the signal bus only carries PlayStarted/Skipped/Completed.
- Confirm the raw-0 reading of "all-unknown key (0)" (deviation 1), since the ticket also fixes the version nibble at 1.
- The other processes still running (cargo clippy and cargo check at 14:04:21, and a python from yesterday) are not mine. I left them alone.
