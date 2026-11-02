REVIEW T-0047 (round 1)
verdict: APPROVE
checks (run by reviewer): fmt pass (exit 0), clippy -D warnings pass (exit 0), tests: `cargo nextest run --workspace` 464 passed / 0 failed (3 slow), 6 skipped; `cargo test -p tinwave-db --test storage_growth` 7/7 passed in each of 3 runs. deny: advisories ok, bans ok, licenses ok, sources ok; only the existing duplicate-crate warnings, none from cpu-time.

acceptance criteria:
- [x] Passes with the machine saturated (CPU ~72 % from other agents): plain `cargo test` run 1: writer 6.7 s CPU, 27.1 s wall; run 2: 6.3 s CPU, 23.5 s wall; while 2 full-workspace nextest runs ran: 6.4 s CPU, 185.6 s wall, 7/7 passed. The two background nextest runs: 464/464 each; their 90-day tests took 102.7 s and 107.5 s wall. Standalone nextest: 69.5–70.8 s wall, passed. Every one of those would have failed the old 60 s gate.
- [x] A deliberate 3x writer slowdown still fails: scratch copy, 1.1 ms spin after `write_batch` in `try_commit` (writer.rs:443): 18.5 s CPU, 68.1 s wall, fails at storage_growth.rs:344 "the writer used 18.46875s of CPU time, budget 15s". Copy deleted, worktree clean.
- [x] fmt, clippy -D warnings, tests, deny pass.

Focus questions:
- "Process CPU minus producer-thread CPU" is sound: the writer is one named thread (writer.rs:186), blocks in `drain.wait` (no spin, writer.rs:372), joined inside `run()`; no other thread is spawned in tinwave-db or by the bus; SQLite runs on the calling thread. Producer-side SQL (snapshot inserts, final TRUNCATE checkpoint) is on the producer thread and subtracted. Threads waiting on the `OnceLock` or `SIMULATION` are parked. Serialization verified in both orders; the process-time start is read after the lock (storage_growth.rs:313).
- Poisoned mutex: guard covers only `run(...)`; determinism asserts run after it is dropped; a panic inside `run` poisons it and `unwrap_or_else(PoisonError::into_inner)` recovers — no cascade.
- The two unlocked tests (burst, catalog text) can overlap under plain `cargo test`: well under 0.5 s.
- cpu-time 1.0.0: MIT/Apache-2.0, ~346 lines, `unsafe` only FFI to GetProcessTimes/GetThreadTimes and clock_gettime with stack out-params (sound); `ThreadTime` is `!Send`, used on one thread; deps libc + winapi 0.3.9 (already in the lock via page_size): one new lock entry. `cargo tree -i cpu-time`: dev-dependency of tinwave-db only, not in the shipped app. No release since 2020 — acceptable for its size.
- Size assertions and printed report unchanged; one `speed:` line appended.

blocking findings:
- none

design check: n/a

non-blocking findings:
- storage_growth.rs:288-301 — I/O slowdowns are invisible to the new check: `synchronous=FULL` (fsync per commit) gave 8.2 s CPU, 118.8 s wall, passes. That regression is caught by `connection::tests::a_new_database_gets_every_pragma_of_database_md` (fails at connection.rs:93). Sleep/latency regressions are covered only by the signal_writer timing tests; checkpoint cost was never covered (harness raises checkpoint limits, sim.rs ~287). Doc comment should say the check measures CPU work only.
- storage_growth.rs:7-8 and the `WRITER_CPU_BUDGET` doc — release measured 3.2 s vs 6.3 s debug, so 15 s is ~4.7x in release and a 3x slowdown would pass a `--release` run; the module doc's "debug is as fast" sentence is wrong for writer CPU. The gate runs debug, so the criterion holds.

proposals:
- P: Writer CPU budget per build profile (`cfg!(debug_assertions)`: 15 s debug, ~8 s release) and fix the "as fast in debug" sentence | why: a `--release` run currently allows ~4.7x | size: S

notes for lead:
- The lead's note "nextest timeouts already catch hangs" is wrong: there is no `.config/nextest.toml`, and nextest's default only flags SLOW at 60 s, never kills. The only hang guard is the per-barrier 60 s assert (sim.rs `barrier`). A `slow-timeout = { period = "60s", terminate-after = N }` override for storage_growth would be a hard backstop.
- Experiments ran on a deleted scratch copy; the worktree is unmodified. Background logs: scratchpad\nx1.txt, nx2.txt.
- T-0048 won't affect this measure (producer CPU is subtracted).

---
Lead (2026-11-03): merge. Correction accepted (no nextest kill timeout exists). Proposal P-0148 and the two
non-blocking items, plus a nextest `terminate-after` backstop for storage_growth, fold into T-0048.
