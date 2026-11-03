REVIEW T-0048 (round 1)
verdict: APPROVE
checks (run by reviewer): fmt pass; clippy -D warnings pass (cached, exit 0); tests: `cargo nextest run --workspace` 480 passed / 0 failed / 6 skipped in 34.8 s (load 32 % before; the three 90-day tests ~33 s each, parallel processes); `cargo test --release -p tinwave-db --test storage_growth` 7/7 in 19.7 s (load 76 % before); deny pass. `git merge-tree --write-tree main t/T-0048-growth-barrier-no-spin` clean (tree 8a1ff7c). Worktree clean at 98d986a.

acceptance criteria:
- [x] Producer-thread CPU drops to a small fraction of wall clock: release run printed "writer thread 3.5 s of CPU time (budget 8 s); generator thread 0.6 s of CPU time; 15.3 s of wall clock" (~4 %). The "before" figure (10.5 s) is the builder's; main doesn't print generator CPU.
- [x] Determinism and sizes: both tests pass under nextest and release cargo test. Exported main via git archive and diffed its release 90-day report against the branch: only the header run-time/barrier-wait figures (12.8/11.6 s main vs 15.3/14.3 s branch) and the speed line differ; every table, size, transaction count (11413) and ratio identical. No path where the backoff changes which rows are written: the barrier exit condition (`inserted + duplicate + rejected >= queued_total`) is unchanged and the generator emits nothing while waiting; the idle writer stays idle up to ~0.5 ms longer; `next_pump` only fires after 2 s without a pull; an idle `maintain()` checkpoint wouldn't change `page_count` (reads through the WAL).
- [x] Release 3x slowdown fails: scratch copy, 600k black_box iterations after `write_batch` in `try_commit` → "the writer used 9.71875s of CPU time, budget 8s" (baseline 3.3–3.5 s, ~2.8x). Headroom on a passing run 8/3.5 = 2.3x.
- [x] nextest override only affects storage_growth: `cargo nextest list --workspace -E 'binary(storage_growth)'` lists exactly the 7 tests; no other binary has that name. Scratch copy with `period = "2s", terminate-after = 1`: the 5 long storage_growth tests TERMINATING then TIMEOUT; `signal_settings a_corrupt_row_fails_closed_and_says_so` (other binary) ran 10.7 s and PASSED — the repo-root config is picked up and doesn't leak. Syntax valid for cargo-nextest 0.9.150. No other profiles/CI/`--profile` uses. 300 s justified.
- [x] fmt, clippy -D warnings, tests, deny pass.

blocking findings:
- none

design check: n/a

non-blocking findings:
- crates/tinwave-db/tests/storage_growth.rs:304-306 — the `writer_cpu` doc still says "the time it spins waiting for the writer at the barriers (sleeping, not spinning, so it is small)"; contradictory and grammatically broken. Suggested: "the generator thread (this one) is read separately (it sleeps at the barriers, so its share is small), subtracted and not asserted."
- crates/tinwave-db/tests/growth/sim.rs:135-137 — "ends at most `BARRIER_POLL_MAX` after the commit" holds only for the requested sleep; the Windows floor is ~0.55 ms and longer under load. "about" would be accurate.
- Idle wall clock +~2.5 s (barrier wait 11.6 s main vs 14.3 s branch); accepted by the lead.

proposals:
- none new.

notes for lead: merge clean against current main (T-0044 touched nothing in tinwave-db/-signals/-core). Load during review 32–96 %. Scratch copies deleted; worktree unmodified.

---
Lead (2026-11-03): merged; the two comment nits fixed by the lead on main (trivial); P-0153 applied to testing.md.
