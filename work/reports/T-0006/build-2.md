HANDOFF T-0006 (round 2)
status: done
branch: t/T-0006-signals-db-writer   commit: 698b78f (work commit 34a9fc4, then merge of main at 975e812; no Cargo.toml/Cargo.lock change)
summary: Both blockers fixed, all non-blocking items done. An idle writer checkpoints a non-empty WAL on the policy interval (policy injectable). signal_event_rec_ts dropped (migration 0001 edited in place, tied to database.md by a test). Realistic fixture (50 000-track library, ctx on every event): 60 B test passes at 39.4 B/event.

acceptance criteria:
- [x] Migrations apply and are idempotent (tests incl. schema_matches_database_md, migrate.rs, connection.rs).
- [x] 256-event batch < 5 ms: write_batch on a DB with 100k realistic events: p50 878/863/867 us, mean 917/863/881 us, p99 1.46-1.49 ms, max 3.5-81 ms (2 checkpoints); criterion mean 0.98-1.11 ms. ~3x faster than round 1 without the index.
- [x] <= 60 B/event: 100 002 events, 6.38 events/play, 50 000-track library, ctx on every event (new id ~every 8 events). Table only 39.4 B/event (test asserts no secondary index); whole file 48.2 B/event, 4 712 KiB. Regression guard 41.0 B.
- [x] Writer RSS < 2 MB: +1 628 / +1 352 / +1 452 KiB warm (baseline ~20.1 MB with the larger fixture).
- [x] Duplicate (device, seq) ignored.
- [x] Shutdown flush loses nothing: existing test + shutdown_drains_more_than_one_batch_that_was_still_queued.

review findings:
1. Idle checkpoint: Worker::run calls maintain() when the wait timed out, the stage is empty and retry_at is None. CheckpointPolicy { soft_limit_bytes, hard_limit_bytes, interval, min_gap } in StoreConfig (defaults 8 MiB / 64 MiB / 60 s / 2 s). Flush clock not made injectable (flush_interval already configurable; no test sleeps more than a few hundred ms; old 2.1 s sleep test gone). Test an_idle_writer_checkpoints_a_non_empty_wal_within_the_interval (interval 300 ms, min gap 50 ms; WAL 0 bytes after). Mutation: idle branch disabled => fails ("never checkpointed (checkpoints: 0, WAL 131872 bytes)").
2. Realistic fixture: Listener uses the 50 000-track REFERENCE_LIBRARY, ctx on every event, counts events/play (doc: 6.38 measured vs catalog §9 ~3.75 => conservative on payload). Sessions stamped by the advanced injected clock. signal_event_rec_ts dropped; schema test updated.
3. 0001_init.sql header points to src/connection.rs; no hard-coded B/event. New the_migration_is_the_sql_block_of_database_md compares every statement with database.md (comments/whitespace stripped).
4. OutOfRange rejected individually (events_rejected, new counter_rows_rejected, also in WriterStats), logged with kind and seq only. Tests values_above_i64_max_are_rejected_one_by_one_and_do_not_wedge_the_batch, a_rejected_value_does_not_wedge_the_writer_stage.
5. Hard limit respects min_gap; test the_hard_limit_also_waits_for_the_minimum_gap.
6. shutdown_drains_more_than_one_batch_that_was_still_queued (default 256; DB fails, 1 000 events pile up, DB recovers, shutdown => 1 001 stored). Mutation finish() -> pull(false) fails (257 vs 1001).
7. an_event_before_its_sessions_start_gets_a_negative_offset_that_round_trips (ts_off = -5000).
8. schema_matches_database_md covers every table (dicts, session, signal_event PK/no rowid/no secondary index, context_snapshot, play + play_rec_ts, rollup_monthly, ui_counter) plus the statement-level comparison.
9. Reject log: kind, seq, fixed reason (variant_name), never values.
Also: slow WAL test replaced by three fast ones.

checks: fmt pass, clippy pass, tests 328 passed / 0 failed / 2 ignored (neither in tinwave-db), deny pass; benches ok.
files changed: crates/tinwave-db/{migrations/signals/0001_init.sql, src/{store,writer,lib}.rs, tests/{common/mod.rs, signal_store.rs, signal_writer.rs, storage_budget.rs}, benches/{write_batch,writer_rss}.rs}
signals: none. Storage: 39.4 B/event table, 48.2 B/event whole file.
deviations: none new (flush clock not injectable; policy and intervals are). proposals: none. questions: none.
