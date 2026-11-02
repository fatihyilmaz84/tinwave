HANDOFF T-0007 (round 2)
status: done
branch: t/T-0007-storage-growth-test   commit: 0f3f2e7
summary: Determinism is scheduling-independent: `Sim::emit` waits at a barrier after every `tinwave_db::ends_a_play` event (Completed, Skipped, Paused, SessionEnded). Test-only `GrowthConfig.preempt_after_pause` (default false, never from env) stalls the producer 3 ms after each Paused; `the_rows_are_the_same_when_the_producer_is_preempted_after_a_pause` (5 days, ≥ 10 pauses) requires identical batch count, sizes, tables, event rows and counter rows. Mutation: barrier skipped for Paused ⇒ fails 3/3 (590 vs 604 transactions); with it passes 3/3.
other items: escapes fixed with one `assert_no_violations` helper; drift guard builds expected text from `CATALOG_CONTEXT`, `CATALOG_COUNTERS`, `CATALOG_DICTIONARIES_BYTES`, `Group::catalog()` + `catalog_row_prefix()` (changing CATALOG_COUNTERS to 6 KB fails); extrapolation docs/report say dictionaries aren't compacted and keep growing until the library is covered.
results byte-identical: 4 378 624 B; context 0.93, counters 0.99, dictionaries 0.98; 48.7 KB/day; 5 years with tiering 16.9 MB.
checks: fmt pass, clippy pass, tests 335 passed / 0 failed / 2 ignored (not this ticket), deny pass; release storage_growth 7 passed, 0 ignored (13.7–24 s file, 14.3 s 90-day run).
files changed: crates/tinwave-db/tests/growth/{sim.rs, mod.rs}, tests/storage_growth.rs
proposals/questions: none
