HANDOFF T-0047 (round 1)
status: done
branch: t/T-0047-storage-test-time-budget   commit: 8c89c37
summary: The 90-day speed check now asserts the writer thread's CPU time, not wall clock. The budget is 15 s against about 6.3 s measured. The writer's CPU stayed at 6.0-6.5 s in every run, while wall clock ranged from 16 s to 109 s depending on machine load. The size assertions and the printed report are unchanged. One new dev-dependency was needed: `cpu-time` (see deviations and questions).

How it works:
- The writer's CPU is the process CPU delta minus the producer (test) thread's CPU delta around `run()`. The producer's CPU is not asserted, because it spins in `yield_now` at the barriers while waiting for the writer and so grows with load. The writer commits with `synchronous=NORMAL` (no fsync), so it is CPU-bound and its CPU time is a faithful measure.
- A new `SIMULATION` mutex plus `run_alone()` keeps the 3-day and preempt tests from overlapping the 90-day run in one process. Under `cargo test` they queue; under nextest each test is its own process and nothing changes.

acceptance criteria:
- [x] Test passes with the machine saturated — proof: while two full-workspace `cargo nextest run --workspace --no-fail-fast` ran (both 464 passed / 0 failed / 6 skipped) and other agents' load was also on the machine, `storage_growth` 7/7 passed. The three 90-day processes reported writer CPU 6.2 / 6.1 / 6.4 s against wall clock 95.8 / 45.9 / 45.7 s. The 95.8 s wall run would have failed the old 60 s gate. Earlier runs under other agents' load: writer CPU 6.0-6.5 s at wall 16.5 / 36.7 / 59 / 89 / 109 s. Idle: about 6.3 s CPU at 16.5 s wall.
- [x] A deliberate 3x writer slowdown still fails — proof: temporary mutation in `crates/tinwave-db/src/writer.rs` `try_commit` (a 1.1 ms CPU spin after each `write_batch`, 11 413 commits). Result: 18.6 s CPU (2.95x of 6.3 s), and the test failed with "the writer used 18.625s of CPU time, budget 15s". The mutation is reverted and is not in the commit. An earlier, cruder 7x mutation also failed (44-49 s).
- [x] fmt, clippy -D warnings, tests, deny — proof: all pass.

checks: fmt pass, clippy pass, tests 464 passed / 0 failed (6 skipped), deny pass (advisories, bans, licenses and sources ok)
files changed: crates/tinwave-db/tests/storage_growth.rs, crates/tinwave-db/Cargo.toml, Cargo.toml, Cargo.lock
signals added/changed: none
ui: none
docs updated: none. `docs/rules/storage-budget.md` and `testing.md` have no wording about the 60 s rule. Module doc comment in `storage_growth.rs` updated.

deviations from ticket:
- Added a dev-dependency, `cpu-time = "1.0"`, in the workspace `Cargo.toml` and `crates/tinwave-db/Cargo.toml` (not in the scope list). Why: the workspace has `unsafe_code = "forbid"`, so a raw `GetThreadTimes` or `getrusage` call is impossible, and no other safe API for CPU time was found. `cpu-time` is MIT/Apache-2.0 and only pulls in `libc` and `winapi`, both already in `Cargo.lock`. `cargo deny check` passes. Cross-platform.
- "wall clock < 60 s" became "writer CPU < 15 s" (`WRITER_CPU_BUDGET`). The old 60 s was about 1.9x the idle wall time; 15 s is about 2.4x the CPU time. An extra report line `speed: writer thread X s of CPU time (budget 15 s); Y s of wall clock, not asserted` follows the unchanged report.
- The writer's CPU is derived as process time minus the test thread's time. Only the test thread and the writer thread run during the `SIMULATION` lock. The tiny volume-burst test is not locked, which adds under 0.2 s at most under `cargo test`.

proposals:
- P: Replace the `yield_now` spin in the growth harness barrier with a short park/sleep | why: the spin burns a core and inflates wall clock under load (the producer waited 88.7 s of a 95.8 s run); this would make the producer's CPU a usable measure too | size: S | evidence: crates/tinwave-db/tests/growth/sim.rs:1063

questions for lead:
- Is the added dev-dependency `cpu-time` (outside the listed scope) acceptable?
- Wall clock is no longer asserted at all. A hang is still caught by the 60 s barrier timeout in `sim.rs` and by nextest's timeouts. Add a loose wall-clock backstop?

---
Lead (2026-11-02):
- `cpu-time` dev-dependency: accepted (dev-only, MIT/Apache-2.0, no new transitive crates, deny passes; the ticket
  asked for a load-independent measure and `unsafe_code = "forbid"` rules out a raw call). Root manifests and
  Cargo.lock don't count as scope overlap (work/README.md).
- No wall-clock backstop: the barrier timeout and nextest timeouts already catch hangs.
- Proposal → P-0144 accepted → T-0048 (S, P2): the spin adds a busy core to every gate run.
