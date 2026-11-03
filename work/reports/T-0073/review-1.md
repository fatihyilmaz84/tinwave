REVIEW T-0073 (round 1)
verdict: APPROVE
checks (run by reviewer): fmt pass; clippy pass (`--workspace --all-targets -D warnings`, Qt env); tests: nextest --workspace 802 passed / 0 failed / 6 skipped (5 slow), exit 0, Summary 89.2 s; deny: advisories, bans, licenses and sources ok, exit 0. Merge with main 560e88a: `git merge-tree` gives 373a7ec with no conflicts. The merged tree differs from the branch only in `work/` files, because main gained only board, report and ticket commits since the base 0cd4e7f. So the branch results above are the merged tree's results.

acceptance criteria:
- [x] Mutation: a per-row loop in a wipe fails a test. Verified by my own mutations in a scratch copy (git archive, its own CARGO_TARGET_DIR). I changed the mtime on every swap and saw `Compiling tinwave-db` each time.
  - Uncapped per-play-row UPDATE loop in `signal_wipe::wipe_kinds`: `a_wipe_runs_a_fixed_number_of_statements_whatever_the_row_count` failed for every category, for signal tiers T0/T1 and for wipe_tiers T0/T1 (e.g. "13 on the small database, 3013 on the large one", "3013 statements, expected 9"). T2/T3 return before `wipe_kinds` runs, which is correct.
  - Per-session-row loop in `wipe_everything`: the "everything" case failed (18 vs 3018).
  - Per-distinct-key loop in `wipe.rs::rewrite_keys`: `a_context_wipe_runs_a_fixed_number_of_statements_whatever_the_row_count` failed for all 9 non-empty cases (16 vs 1210), and the signal test failed for wipe_tiers T1-T3 (e.g. 18 vs 6018).
  - The count does not depend on row count. Every case builds a fresh small and a fresh large fixture (+3 000 rows in all 9 tables, 3 000 distinct context keys). It requires identical SQL lists and an exact pinned length, and asserts that the large database really had more rows to touch. A per-kind loop (same on both sizes) is caught by the pinned length.
  - The trace (SQLITE_TRACE_STMT) captures every statement on the wiper connection: BEGIN IMMEDIATE, COMMIT, the epoch PRAGMAs and both `wal_checkpoint` PRAGMAs are in the pinned counts. busy_timeout is a C API call, not a statement.
- [x] Mutation: a lock left open after a wipe fails a test. I added `BEGIN IMMEDIATE` after the checkpoint, one entry point at a time:
  - wipe_signal_categories: Category mode failed ("a batch sent after the wipe returned was not stored (5.43 s later) … database is locked").
  - wipe_tiers: Category passed, Tiers failed (5.55 s).
  - wipe_everything: Category and Tiers passed, Everything failed (5.55 s).
  - wipe_fields (context): the context reference test failed (5.55 s).
  - wipe_signal_tiers (no contention test uses it): still caught, by `several_tiers_at_once_and_a_repeated_wipe` and by the statement-count test, since the leaked BEGIN is a traced statement. So every entry point is covered.
  - The wiper is still alive when the second batch is sent (it is a local of the loop or Fixture), so the test does catch the leak.
- [x] 5 s after-wipe bound under load. I ran 16 signal + 16 context reference-test copies in parallel, 3 rounds, the third with 8 extra statement-count copies (40 processes):
  - 96/96 passed; the 8 extra copies passed 8/8.
  - I instrumented the bare after-wipe batch (eprintln in the scratch copy only). Over 128 samples in rounds 2-3: average 232 / 407 ms, max 1.18 s / 1.77 s.
  - So the margin is about 2.8x at 40 processes. A real leak fails at about 5.4 s, through the busy timeout and the error, not the slack assert.
  - Wipes reached 12.9 s, and after_wipe was 0 in every run, which confirms again why the first batch could never show a leak.
- [x] SECTIONED set-equal. `signal_wipe.rs:530-551` checks both directions. Adding `SignalKind::Favorited` to SECTIONED (scratch) failed with "Favorited is sectioned and belongs to QueueLibrary".
- [x] privacy-and-signals.md:79-83 and wipe.rs:363-367 reader-case wording. The writer case keeps "at most 60 s". The reader case now says the content stays for as long as the reader is open, and the first checkpoint after the read transaction ends removes it. This matches `store.rs:474-496`: `last_checkpoint` is set on every attempt, so a busy TRUNCATE is retried one interval later. One caveat applies to both cases and predates this branch: the interval path needs `idle`.
- [x] fmt, clippy, nextest, deny pass (see checks).

Lead-accepted out-of-scope items, verified:
- `connection_for_tests` (wipe.rs:219-225): the only callers are `tests/common/mod.rs:471,483`. Nothing in `src/` or `apps/` calls it, and nothing in the workspace depends on tinwave-db (`cargo tree -i tinwave-db`).
- The `trace` feature is dev-only: `cargo tree -e normal,features -i rusqlite` shows bundled/cache/hashlink/modern_sqlite only, and `trace` appears only with dev edges. Cargo.lock is not in the diff.

blocking findings:
- none

design check: n/a

non-blocking findings:
- crates/tinwave-db/tests/signal_wipe.rs:1340-1393 (`a_wipe_runs_a_fixed_number_of_statements_whatever_the_row_count`) — the test is now the longest in the workspace. It took 84.6 s in my full nextest run, against 78-81 s for the storage_growth tests, and it was the last test to finish (suite 89 s). Alone it takes 11-13 s.
  - No timeout risk: `.config/nextest.toml` only sets terminate-after for `binary(storage_growth)`, and for everything else SLOW is reported, never killed.
  - The comment there ("the longest test in the workspace") is now stale.
  - About half the cost is `Fixture::new` → `populate()` running about 100 autocommit inserts on a default (synchronous=FULL) connection, 32 times. Wrapping `populate` in one transaction cut the test from 11.0 s to 6.4 s in my scratch run.
- crates/tinwave-db/tests/common/mod.rs:550-565 — the after-wipe batch is "milliseconds" on an idle machine, but up to 1.77 s at 40 processes. The 5 s bound holds with about 2.8x margin; no failures in 96 runs. Worth watching if the machine load grows further.

proposals:
- P: Make the signal-wipe statement-count fixture cheap (populate in one transaction, or build the small and large databases once and copy the files) | why: it is now the slowest test and on the suite's critical path (84.6 s under load); one transaction alone roughly halves it | size: S

notes for lead:
- Merge: clean, and main's only changes since the base are under `work/`, so it can merge as is.
- T-0074 (remove `connection_for_tests`) is unaffected. My mutations show the statement-count helper also catches a leaked lock on every entry point, so the T-0074 rewrite should keep the BEGIN/COMMIT/checkpoint statements in the count.
- Cleanup: I created only `C:\Users\FSYY\AppData\Local\Temp\claude\D--dev-Tinwave\53bf9027-c019-4dd4-82f1-5a72c6705b0c\scratchpad\rev73` (sources, target dir, binaries, outputs, scripts). I listed it and then deleted it; it is gone.
- The worktree is unchanged at f4d1c00 with a clean status. None of my test or cargo processes are running; the cargo processes still visible belong to the other worktrees.
