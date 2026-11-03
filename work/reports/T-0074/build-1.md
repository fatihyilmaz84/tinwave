HANDOFF T-0074 (round 1)
status: done
branch: t/T-0074-test-only-connection-access   commit: 0304878 (two commits on f664b90: fed10a0, 0304878)
summary: `connection_for_tests` now exists only with a new `test-support` feature of tinwave-db, which only tinwave-db's own self dev-dependency enables. A consumer with default features cannot compile a call to it. `SignalStore::write_batch` has a pinned statement count (`6 + 8n` for n plays, `3 + 3n` for n counter rows, 36 for a replayed batch of 10 plays), identical on a database holding 3 plays and one holding 3000. The statement-count fixtures are built once and copied, so each of the two wipe cost tests dropped from 10-36 s to 1-3 s when run alone.

acceptance criteria:
- [x] The default build of tinwave-db exposes no raw Connection; `cargo build -p tinwave-desktop` cannot reach one through it — proof:
  - `cargo tree -p tinwave-desktop -i tinwave-db` gives "package ID specification `tinwave-db` did not match any packages": the desktop app does not depend on tinwave-db today, so a desktop build compiles none of it.
  - `cargo tree --workspace -e normal,build,features -i tinwave-db` shows only `feature "default"`. `test-support` appears only under `[dev-dependencies]` (`-e all,features`), where the self edge enables it.
  - A throwaway consumer crate (path dependency on tinwave-db, default features) calling `wiper.connection_for_tests()` fails with `error[E0599]: no method named connection_for_tests found for struct SignalWiper`. The same crate with `features = ["test-support"]` compiles. The scratch crate is deleted.
  - Chosen design: feature plus self dev-dependency, not an in-crate `#[cfg(test)]` module, because the counting helper lives in the integration tests (`tests/common`, shared with benches), which cannot see `#[cfg(test)]` items of the library.
- [x] write_batch statement count pinned; a per-row mutation fails it — proof:
  - Test `signal_store::a_batch_runs_a_statement_count_set_by_the_batch_whatever_is_already_stored`. Each batch runs on a fresh store with a cold cache, once on a database with 3 stored plays and once on one with 3000.
  - It pins these counts exactly:
    - n plays (1, 2, 10, 100), each a PlayStarted and a Completed of a new recording: `6 + 8n`.
    - n counter rows (1, 10): `3 + 3n`.
    - A replayed batch of 10 plays: 36.
  - It requires equal counts on both databases, first statement `BEGIN IMMEDIATE`, last `COMMIT`.
  - `an_empty_batch_runs_no_statement` pins 0 statements for an empty batch.
  - Mutations, each reverted with `git checkout` (mtime changes, `Compiling tinwave-db` seen):
    - `SELECT count(*) FROM signal_event` then one `SELECT 1` per stored row in `write_batch`: FAILS (21 statements on the small database vs 6015 on the large one, expected 14; replay 63 vs 6057).
    - Same loop per stored `play` row inside `play::write_play`: FAILS (18 vs 3015 for 1 play, and 6156 vs 305 856 for 100).
    - One extra statement per event: FAILS (expected 14/22/86/806, got 16/26/106/1006).
- [x] T-0073 guarantees kept; its mutations re-run — proof:
  - The statement lists still come from `trace_v2` on the wiper connection, so BEGIN IMMEDIATE, COMMIT, the epoch pragmas and both `wal_checkpoint` pragmas are counted. The pinned lengths are unchanged (8, 9, 16 and so on).
  - Per-row `UPDATE play` loop in `signal_wipe::wipe_kinds`: `a_wipe_runs_a_fixed_number_of_statements_whatever_the_row_count` FAILS for every category and tier ("13 on the small database, 3013 on the large one", "3013 statements, expected 9").
  - Leaked `BEGIN IMMEDIATE` after `checkpoint_best_effort` in `wipe_signal_categories`: both the signal and the context cost tests FAIL ("9 statements, expected 8"). I did not re-run the slower contention reference tests for this mutation; the leaked lock is caught by the statement count.
- [x] P-0301: fixtures are cheap — timings below.
- [x] fmt, clippy, nextest, deny pass.

timings (P-0301):
- Alone, before (base f664b90, repeated runs): `signal_wipe::a_wipe_runs_a_fixed_...` 25-36 s, `context_wipe::a_context_wipe_runs_a_fixed_...` 10-14 s. T-0073's review measured 11-13 s for the signal test on an idle machine; this machine was already busy.
- Alone, after (repeated runs): signal test 2.1-2.2 s, context test 0.8-2.5 s. The first run of a freshly built binary took 31 s once (new .exe, probably a virus scan); later runs 2 s.
- Same load, side by side (base and mine as copied .exe files, 4 processes started together, 3 rounds):
  - Round 1: base 47 s / 36 s, mine 34 s / 33 s (cold binaries).
  - Round 2: base 148 s / 8.5 s, mine 3.7 s / 2.8 s.
  - Round 3: base 145 s / 83 s, mine 4.3 s / 2.2 s.
  - The signal test no longer sits on the critical path under contention.
- In full runs (the machine was shared with other agents, so the numbers vary a lot):
  - T-0073 review: 84.6 s of an 89 s suite.
  - Mine, run 1: signal test 73 s, suite 181 s.
  - Mine, final run: signal test 24.8 s, context test 49.7 s, suite 120.7 s, with the `storage_growth` tests taking about 160 s.
  - I could not get a clean full-run before/after on this machine because the load differs between runs. The side-by-side figures above are the controlled comparison.

checks: fmt pass, clippy pass (`--workspace --all-targets -D warnings`, Qt env), tests 814 passed / 0 failed / 7 skipped (final `cargo nextest run --workspace --no-fail-fast`, exit 0, 120.7 s), deny pass (advisories, bans, licenses and sources ok).
Earlier runs of the full suite on this branch:
- Run 1 (`--no-fail-fast`): `tinwave-desktop::image_provider cancel_storm_with_requests_in_flight_then_quit_exits_cleanly` failed ("no requests at all" in a 150 ms storm) with 813 passed. It also failed once alone, and passed in the final full run. This diff touches nothing under `apps/`; it looks like a load flake (T-0071 is on it), not caused by this change.
- A crate-only `-p tinwave-db` run had 4 `storage_growth` tests hit the 300 s terminate-after under machine saturation. I did not re-investigate; the final full run passed them.

files changed:
- crates/tinwave-db/Cargo.toml
- crates/tinwave-db/src/wipe.rs
- crates/tinwave-db/src/store.rs
- crates/tinwave-db/tests/common/mod.rs
- crates/tinwave-db/tests/context_wipe.rs
- crates/tinwave-db/tests/signal_store.rs
- crates/tinwave-db/tests/signal_wipe.rs
- Cargo.lock (one line: the self dev-dependency edge `tinwave-db` in tinwave-db's dependency list; unavoidable with a self dev-dependency)

signals added/changed: none
ui: none
docs updated: none (no doc mentions `connection_for_tests` or `trace_v2`; docs/ untouched)

deviations from ticket:
- Cargo.lock changed by one line (the self-edge), outside `crates/tinwave-db/` but a direct consequence of the self dev-dependency.
- Added `SignalStore::connection_for_tests`, also gated by `test-support`, so `write_batch` can be traced. This is the same kind of access as the wiper's.
- Fixed a pre-existing flake in `signal_wipe::a_batch_that_outwaits_the_busy_timeout_is_stored_by_the_retry`:
  - It failed 4 times out of 6 here, on my branch only.
  - On base f664b90 it passed 2 of 2; I did not run it on base under the loaded conditions that made mine fail.
  - Cause: the test released its lock after a fixed `BUSY_TIMEOUT + 1 s`. SQLite's 5 s busy timeout is the sum of its back-off sleeps, and on Windows under load each sleep runs longer, so the first attempt was still waiting at 6.1 s and succeeded (`attempts: 1`).
  - Fix: the holder thread now commits as soon as the first attempt has given up with a busy error, using a new `write_batch_when_unlocked_counting` helper in `tests/common`. It passed 4 of 4 afterwards.
- The statement tests' failure messages now print the first 12 and last 4 statements through `common::shown`, not the whole list. A per-row mutation otherwise printed up to 300 000 lines and made the failing run take minutes.

proposals:
- P: Update the stale T-0074 mention in `.config/nextest.toml` (lines 5-6: "T-0074 makes it cheaper") | why: the signal-wipe statement-count test is now about 2 s alone, so the comment is out of date; it is outside this ticket's scope | size: S | evidence: .config/nextest.toml:5-6
- P: Make `cancel_storm_with_requests_in_flight_then_quit_exits_cleanly` robust to a slow QML start | why: it failed with "no requests at all" in a 150 ms storm on a loaded machine (full run 1, and once alone) | size: S | evidence: apps/tinwave-desktop/tests/image_provider.rs:161 (T-0071 area, not touched here)
- P: Timer-resolution-independent busy-timeout assumptions elsewhere | why: the T-0073 contention tests assume 5 s of wall time equals the SQLite busy timeout, which does not hold on loaded Windows (I measured 6.1-6.4 s); worth a grep for similar `BUSY_TIMEOUT + n s` constants | size: S | evidence: crates/tinwave-db/tests/signal_wipe.rs (the fixed test) and `common::AFTER_WIPE_SLACK`

questions for lead:
- None. Cleanup: I created only scratch files under the session scratchpad (the consumer crate, copied .exe files, scripts) and deleted them. `target/` in the worktree holds build output and `nextest-full.txt` / `nextest-exit.txt`. The cargo, nextest and image-stress processes still visible belong to the T-0071 worktree and others, not to this ticket; none of mine are running.
