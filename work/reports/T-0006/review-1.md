REVIEW T-0006 (round 1)
verdict: CHANGES_REQUESTED
checks (run by reviewer): fmt pass, clippy pass (`--workspace --all-targets -D warnings`), tests 265 passed / 0 failed / 1 ignored (tinwave-test-alloc doctest), deny pass. Benches: write_batch gate ok p50 2.74 ms, mean 3.33 ms, p99 19.0 ms, max 53.9 ms, 14 checkpoints (criterion mean 4.64 ms, noisy). writer_rss ok: +1352 KiB warm, +2336 KiB cold.

acceptance criteria:
- [x] Migrations apply on an empty DB and are idempotent — tests + migrate.rs unit tests + connection.rs pragma tests; SQL block of main's database.md diffed against 0001_init.sql: only one comment differs.
- [x] 256-event batch write < 5 ms — p50 2.74 ms, mean 3.33 ms on a DB holding 100k events.
- [ ] 100 000 realistic events ⇒ ≤ 60 B/event incl. index — the fixture never sets `ctx`; with it the reference listener measures 60.0+ B (blocking 2).
- [x] Writer adds < 2 MB RSS — +1352 KiB warm; cold +2.3 MB (SQLite code, already loaded by library.db in the app; accepted as the lead did).
- [x] Duplicate (device, seq) ignored — `a_duplicate_device_seq_is_ignored`.
- [x] Shutdown flush loses nothing accepted by the bus — tests; mutating `finish()` to skip the final drain fails 1–3 writer tests per run.

all-or-nothing (mutations in a scratch copy): one `write_batch` = one IMMEDIATE transaction covering dictionary rows, session rows, events, counter upserts (store.rs:244-326); every `?` before commit drops/rolls back; failed COMMIT also rolls back; only infallible `publish()` after commit. Mutation "commit after events, new transaction for counters" fails signal_store.rs:624 and :703 (trigger test is real). Mutation "insert into the LRU before commit" (dict.rs:129) fails both (:630, :719); rolled-back ids safe (`fresh` maps dropped unpublished). Mutation "clear the stage when a commit fails" fails `a_failing_database_is_retried_and_nothing_is_lost_or_counted_twice`.

staging: stage kept on failed commit; writer stops pulling while `retry_at` is set (writer.rs:350-359), memory bounded; shutdown during an outage drains the bus, tries once, reports `events_lost` with the error; `Stage::write` can't fail so the bus's retained-batch backoff never applies (writer's `retry_at` replaces it, as accepted).

checkpoint, sessions, errors, deps: checkpoints never inside a commit (`wal_autocheckpoint=0`, tested); with a reader holding a transaction `checkpoint_if_due` returned busy in 180 ms (debug) without waiting, writer kept committing; session-by-uuid survives restart; `started_ts + ts_off` round-trips; small negative `ts_off` stored as 1-byte signed ints; `DbError` keeps rusqlite source (downcast asserted); rusqlite 0.41 bundled + cache, tempfile dev-only, tracing std-only; scope clean, no TODOs.

blocking findings:
1. crates/tinwave-db/src/writer.rs:364-367 and :407-410 — "checkpoint every 60 s while the WAL is non-empty" is not implemented for an idle writer: `maintain()` runs only right after a commit and `due()` is false when the stage is empty. Scenario: user pauses; `Paused` is flushed but < 60 s after the last checkpoint, so `maintain` skips it, and nothing calls it again; with `synchronous=NORMAL` that commit stays un-fsynced until the next commit or exit (potentially hours), so a power loss can roll back committed play ends, contradicting database.md and store.rs:42-46. Evidence: scratch copy with `WAL_CHECKPOINT_INTERVAL` = 1 s: after the `Paused` commit WAL 140 112 B; after 5 s idle still 140 112 B, 0 checkpoints. Fix: when the stage is empty, `retry_at` is None and the wait timed out, call `maintain()` (already rate-limited by the 2 s gap); make the interval injectable; add a writer-level test that an idle writer checkpoints a non-empty WAL.
2. tests/common/mod.rs:314-328 (`Listener::push`, `ctx: None`) and tests/storage_budget.rs:30 — the realistic-events measurement leaves out `ctx` (context-engine.md:20-21: every T0 event gets a ctx id; T1 on by default) and uses a 5 000-track library (testing.md reference user: 50 000). Measured in a scratch copy with a new snapshot id about every 8 events (~60/day): branch fixture 57.3; 5k library + ctx 59.6 (58.5 guard fails); 50k library + ctx 60.0+ (60 B test fails); 5–8 h sessions 56.8. The budget has ~0 B headroom. Fix: realistic `ctx`, reference library size; then a lead decision to get back under 60 B (e.g. ctx only on PlayStarted or on change), or accept ~60 B.

design check: n/a

non-blocking findings:
- 0001_init.sql:2,19 stale comments ("see src/pragma.rs" → connection.rs; "measured 54 B"). The checksum covers comments, so after merge any edit breaks every existing dev signals.db with `MigrationChanged` — fix now with the final figure.
- store.rs:281,289,316-317 — an `OutOfRange` (seq, ctx, or a counter value above i64::MAX) fails the whole stage permanently and the writer retries forever while the bus drops everything after it; reject per event like unencodable payloads, or make `ContextSnapshotId` fit i64. Nothing produces such a value today.
- store.rs:375 — the 64 MiB case skips the 2 s minimum gap; with a long-lived reader every batch retries a TRUNCATE checkpoint that copies and fsyncs then reports busy (180 ms at 9.5 MB).
- signal_writer.rs:94 — the shutdown test uses `flush_events = 100_000`, so a bug stopping the final drain at 256 events (`pull(false)`) passes (checked by mutation); add a case with the default 256 and > 256 events queued.
- No test for a negative `ts_off`.
- tests/common/mod.rs:260 doc says ~9 events/play; the model produces ~6.4; catalog §9 ~3.75 (heavier on payload, lighter on ctx).
- signal_store.rs:90 `schema_matches_database_md` covers signal_event, session, dict_entity, ui_counter only, not play, rollup_monthly, context_snapshot (hand-checked; only the comment differs).
- store.rs:258-263 — the warn log includes the encode error, and `SignalsError` "value {value} is not a valid {what}" can carry a payload field value (rust.md:37: never log raw payloads); log kind, seq and error variant only.

proposals:
- P: Decide how `ctx` is stored to meet the 60 B budget | size: S
- P: Make writer and store timing injectable (checkpoint interval and gap, flush clock) | size: S

notes for lead:
- Blocking 2 needs a schema/budget call before the builder can finish.
- Scratch copy: session scratchpad rev-t0006-r1 (contains REV_* probes; deletable).
- The reviewer overwrote the old scratchpad `mut` directory (earlier session artifacts); regenerate if anything relies on it.
- No changes to D:\dev\Tinwave or any worktree.

---
Lead (2026-11-02): blocking 2 decided — drop `signal_event_rec_ts` (no consumer; compaction scans in (device, seq)
order, taste/Insights read `play`); ctx stays on every event; database.md updated. Round 2 (last allowed) sent with
both blockers, injectable timing, and all non-blocking items.
