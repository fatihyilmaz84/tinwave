HANDOFF T-0064 (round 1)
status: done
branch: t/T-0064-signal-wipe-per-tier   commit: 104f688
summary: Added `SignalWiper::wipe_signal_tiers(&[Tier])` and `SignalWiper::wipe_tiers(&[Tier])`. `wipe_tiers` wipes a tier's signal events, their derived data and the T-0060 context fields in one transaction. The derivation sources in signal_wipe.rs are now signal kinds instead of categories, and a category wipe gives exactly the same result as before. The contention tests now retry a locked batch the way the writer thread does and wait on conditions, not on the 5 s busy timeout. The TRUNCATE-busy log and the privacy doc now name both causes (writer or reader holding the lock).

acceptance criteria:
- [x] Per tier: exactly that tier's events and derived data go, tested per tier. Proof: `signal_wipe::wiping_a_tier_deletes_exactly_its_events_and_the_data_derived_from_it`. It covers T0, T1, T2 and T3 with literal expected rows and reports. T0 gives 36 events, 3 counters, 3 plays and 4 rollups. T1 gives the 2 events (kinds 69 and 81) plus the score reset. T2 and T3 leave a full dump identical. `several_tiers_at_once_and_a_repeated_wipe` checks that wiping all tiers leaves the same dump as wiping all categories.
- [x] "Wipe tier" is one call that includes the T-0060 context tier wipe. Proof: `a_tier_wipe_is_one_call_for_the_signals_and_the_context_of_the_tier`. It builds the keys with the typed builders and checks T2, T1+T3 and T0.
- [x] Atomic, and the writer keeps working. Proof: `a_failed_tier_wipe_changes_nothing_and_works_once_the_fault_is_gone`. It injects a failure in the last statement of the context part (combined case) and of the signal part (signal-only case), and a full 12-table dump is identical afterwards. The contention helpers are reused: `a_write_batch_during_a_wipe_of_the_reference_tables_is_stored` now has Category, Tiers (T1+T2, with the 131 400 plays' keys rewritten) and Everything cases, and each asserts that the probe saw the lock.
- [x] P-0272 / P-0280: the contention tests assert a measured bound, not only the busy timeout. Proof:
  - `tests/common/mod.rs` has `write_batch_when_unlocked`. It offers the batch again on "database is locked", as the writer does with a failed commit, and gives up only if the batch is not stored within `AFTER_WIPE_SLACK` (5 s) after the wipe returned. Any other error is returned at once.
  - `a_batch_that_outwaits_the_busy_timeout_is_stored_by_the_retry` pins this deterministically: a 6 s lock, at least 2 attempts, and the batch is stored.
  - The reference tests also assert `waited <= wipe_time + slack`.
  - `context_wipe.rs` uses the same helper.
- [x] P-0277: `the_writer_thread_keeps_working_while_everything_is_wiped` now waits on conditions. It emits until 100 events are stored, holds the wipe open with a burn trigger, keeps emitting until the wipe returns, then emits 50 more. Every wait has a `LOAD_BUDGET` of 120 s.
  - The old fixed 600-event run could wipe everything and leave `remaining == 0`.
  - A failed commit is accepted only if `wipe_time > BUSY_TIMEOUT`.
  - `events_lost == 0`, `inserted == accepted` and `wiped + stored == accepted` still hold.
  - It passed 48/48 with 16 copies plus 16 reference tests at the same time. The wipe took 1.35 s on average, 2.5 s at most.
- [x] P-0281: the TRUNCATE-busy log and the privacy doc name both causes. `checkpoint_best_effort` now returns `Checkpoint::{Done, Busy{main_file_current}, Failed}`. `main_file_current` is true when PASSIVE copied every frame, i.e. when only the writer was in the way. Proof:
  - `wipe::tests::a_checkpoint_blocked_by_the_writer_reports_a_current_main_file`
  - `wipe::tests::a_checkpoint_blocked_by_a_reader_reports_a_stale_main_file`
  - The privacy doc's wipe paragraph describes both causes and the per-tier wipe.
- [x] fmt, clippy, nextest, deny pass (see checks).

Contention numbers (debug, this machine, 16 threads):
- Quiet: the batch waited 0.65-0.87 s, and the wipe took about 0.9-1.0 s.
- 16 parallel copies of the reference test (3 rounds, 48 runs per case):

| case | wipe median / max | batch wait median / max |
|---|---|---|
| Category | 5.59 s / 9.03 s | 2.06 s / 5.95 s (a run with 2 attempts, which the retry saved) |
| Tiers | 6.51 s / 10.36 s | 1.79 s / 3.63 s |
| Everything | 6.17 s / 10.45 s | 1.18 s / 1.69 s |

- 16 reference copies plus 16 context copies at once, which is the 32-heavy-process load of the T-0065 review:
  - signal test: 48/48 pass. Category batch wait median 2.82 s, max 7.94 s (2 attempts). Tiers max 4.68 s. Everything max 4.07 s.
  - context test: 48/48 pass, wait median 0.98 s, max 4.12 s.
  - The T-0065 reviewer saw 7/48 failures on the branch and 3/48 on main under that load.
- In about 480 contended runs the batch was stored before the wipe returned (after-wipe 0 s, max 0.01 s). That is why `AFTER_WIPE_SLACK` is 5 s: it leaves room for a stall and still catches a lock that outlives the wipe.
- I did not add an absolute bound on the wipe time. I tried calibrating it against the burn time (a CPU-bound join). The wipe-to-burn ratio went from 1.6 to 19 under load, so any such bound would be flaky or toothless.

checks: fmt pass, clippy pass, tests 797 passed / 0 failed (6 skipped; 4 reported slow, all storage_growth), deny pass (advisories, bans, licenses, sources ok)
Mutations tried, each restored and checked with `cmp`:
- Tier filter replaced by "all kinds": the per-tier test, the combined test and the atomic test fail.
- `score_sum` with no sources: the same three fail.
- The context part of `wipe_tiers` skipped: the combined and atomic tests fail.
- `main_file_current` forced true: the reader checkpoint test fails.
- No retry in the helper: `a_batch_that_outwaits_the_busy_timeout_is_stored_by_the_retry` fails.
- Wipe epoch bump disabled: the warm-cache test and the writer-thread test fail.

files changed:
- crates/tinwave-db/src/signal_wipe.rs
- crates/tinwave-db/src/wipe.rs
- crates/tinwave-db/tests/signal_wipe.rs
- crates/tinwave-db/tests/common/mod.rs
- crates/tinwave-db/tests/context_wipe.rs
- docs/rules/privacy-and-signals.md

signals added/changed: none (a wipe only deletes)
ui: none
docs updated: docs/rules/privacy-and-signals.md (wipe paragraph only: per-tier wipe, both causes of a busy checkpoint, retry of a batch that waits longer than 5 s; the "not implemented yet" sentence is removed). Module docs in signal_wipe.rs and wipe.rs are updated.

deviations from ticket:
- I touched `tests/common/mod.rs` and `tests/context_wipe.rs`, which are outside the listed scope. The ticket text itself names the contention helpers, and both contention tests share `during_a_wipe`, whose `during` closure now receives the wipe-end instant. Please confirm that is acceptable.
- Derived-data sources changed from categories to kinds in signal_wipe.rs, so a tier, which cuts across categories, clears exactly what its own signals feed. Category-wipe behaviour is unchanged: all pre-existing tests pass unmodified.
- `wipe_tiers` returns a tuple `(SignalWipeReport, WipeReport)` instead of a new public type, because adding a public type would need `lib.rs`, which is out of scope.
- Cost discipline is unchanged: a tier wipe runs the same fixed set of single-scan statements as a category wipe (2 deletes, 1 play update, 1 rollup delete, 1 rollup update), plus the 2 context rewrites for the combined call, with no per-row or per-key loops. There is no test that counts statements.

Decisions the lead should know about:
- `score_sum` is reset by any signal wipe, including the T1 wipe, which deletes only the two T1 events. This follows the existing "blend of every signal" rule. Say so if a T1 wipe should leave it alone.
- Wiping T0 deletes the play records, because `PlayStarted` is T0.

proposals:
- P: Pair the play writer with a hot-window guard for stale starts that never got a row | why: the T-0065 reviewer noted that an orphan outcome can still pair with an old start without a wipe | size: S | evidence: work/reports/T-0065/review-1.md (non-blocking finding 2)
- P: Make the Signal Center flush the writer before "wipe now" | why: events buffered in the bus at wipe time are stored after the wipe (by design, tested), so the last 2 s survive the wipe | size: S | evidence: work/reports/T-0061/review-1.md notes for lead
- P: Expose the busy-checkpoint outcome as a counter in Diagnostics | why: `Checkpoint::Busy { main_file_current }` is only logged today, and the user has no way to see that old content still sits in the WAL file | size: S | evidence: crates/tinwave-db/src/wipe.rs checkpoint_best_effort

questions for lead:
- Is the scope extension to tests/common/mod.rs and tests/context_wipe.rs fine? Both are test-only.
- Should a T1 wipe reset `score_sum`? See "Decisions the lead should know about" above.
- The cargo, clippy and python processes in the process list belong to the other builders. All of mine have finished, and I left nothing running.
