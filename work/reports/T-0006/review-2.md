REVIEW T-0006 (round 2)
verdict: APPROVE
checks (run by reviewer on 698b78f): fmt pass, clippy pass, tests 328 passed / 0 failed / 2 ignored (neither in tinwave-db), deny pass. Benches: write_batch gate ok (p50 922.6 µs, mean 928 µs, p99 1.57 ms, max 15.3 ms, 2 checkpoints; criterion mean 1.13 ms); writer_rss ok (+1600 KiB warm; +2588 KiB cold, same accepted caveat).

acceptance criteria:
- [x] Migrations apply on an empty DB and are idempotent — tests incl. `schema_matches_database_md` (every table). `the_migration_is_the_sql_block_of_database_md` is a real guard: changing `ts_off INTEGER NOT NULL` to `ts_off INTEGER` in database.md fails it; adding a `CREATE INDEX` fails it; comment-only edits pass.
- [x] 256-event batch < 5 ms — p50 0.92 ms on a 100k-event DB (50k library, ctx on every event).
- [x] 100 000 realistic events ≤ 60 B/event — reviewer's own dbstat breakdown: 0 NULL ctx rows, 11 723 distinct ids (~8.5 events per id), 50k library (max recording id used 7 550), 457 sessions, 6.38 events/play; `signal_event` 961 pages ⇒ 39.4 B/event; all objects + 2 pointer-map pages = page_count 1178, freelist 0, no hidden autoindex ⇒ 48.2 B/event whole file. Honest.
- [x] Writer RSS < 2 MB — +1600 KiB warm.
- [x] Duplicate (device, seq) ignored.
- [x] Shutdown flush loses nothing — incl. `shutdown_drains_more_than_one_batch_that_was_still_queued` (default 256, 1 001 queued).

round-1 blockers:
1. Idle checkpoint fixed (writer.rs:379-384). Reviewer scenario (Paused play end, then idle; default min_gap, 1 s interval): WAL 131 872 B → 0 after 2.0 s, 1 checkpoint, row intact. With 60 s default, within ~interval + stop_poll. Disabling the idle branch fails the builder's test and the reviewer's probe.
2. Storage realism resolved under the lead decision (rec index dropped, ctx on every event; migration enforced against database.md).

mutations (scratch copy rev-t0006-r2), all killed: commit-early; LRU-before-commit; clear-stage-on-failure; `finish()` with `pull(false)` (survived in round 1, now killed); hard limit ignoring `min_gap`.

other round-1 items verified: OutOfRange rejected one at a time (store.rs:281-286, :333-340; tests); hard limit waits for min_gap (store.rs:414-417); negative ts_off round-trips (−5 000); reject logs carry no values (store.rs:478-496); scope clean (crates/tinwave-db/, Cargo.toml, Cargo.lock).

blocking findings:
- none

design check: n/a

non-blocking findings:
- Writer flush timing still reads `Instant::now` (P-0080 covers it; policy injectable; only the 300 ms idle test sleeps).
- Writer RSS vs cold baseline ~+2.6 MB (accepted caveat).
- benches/write_batch.rs:5-6 doc line > 100 columns (cosmetic).

proposals: none

notes for lead:
- Safe to merge. Headroom large (39.4 B table, 48.2 B file); recording ids above 32 767 add ~1 byte on those events — inside budget.
- Scratch: session scratchpad rev-t0006-r2/ and mutate_r2.py (deletable).
