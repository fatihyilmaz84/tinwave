HANDOFF T-0048 (round 1)
status: done
branch: t/T-0048-growth-barrier-no-spin   commit: 98d986a
summary: The growth harness barrier now sleeps with a 50 to 500 us backoff instead of spinning. Generator-thread CPU for the 90-day run fell from 10.5 s to 0.7 s (release) and from 13.5 s to 1.5 s (debug). `WRITER_CPU_BUDGET` is now 15 s in debug and 8 s in release, and the docs are corrected. `.config/nextest.toml` adds a `terminate-after` backstop for the storage_growth binary only. A temporary 3x writer slowdown fails a release run. One cost: idle wall clock is about 3 s longer (see deviations).

acceptance criteria:
- [x] Producer-thread CPU drops to a small fraction of wall clock. The `speed:` line now also prints the generator thread's CPU (same ThreadTime as the writer measurement).

  | Profile | Producer CPU before | Producer CPU after | Wall clock before | Wall clock after |
  |---|---|---|---|---|
  | release | 10.5 s | 0.7 s (about 5%) | 11.3 s | 14.1-15.0 s |
  | debug | 13.5 s | 1.4-1.5 s (about 9%) | 14.2 s | 16.9 s |

  Release "after" is 0.7 s in every run (1.0 s with the 20/100 us variant); debug 1.5 s and 1.4 s. Machine load 60-100 % before the release baseline, 77 % before the debug baseline, 35-86 % for the "after" runs (Ryzen 7800X3D, 16 logical CPUs). Writer CPU unchanged: 3.1-3.6 s release, 5.8-6.0 s debug.
- [x] `the_same_seed_gives_the_same_sizes` and `the_rows_are_the_same_when_the_producer_is_preempted_after_a_pause` pass; sizes unchanged: storage_growth 7/7 under `cargo test` and nextest (480 passed / 6 skipped workspace-wide). Full 90-day report of main vs branch in release diffed: only the timing lines differ.
- [x] `cargo test --release`: a 3x writer slowdown fails: temporary 560 us wall spin after `write_batch` in `try_commit` → "the writer used 10.015625s of CPU time, budget 8s" against a 3.2 s baseline (3.1x). The old 15 s budget would have let it pass. Mutation reverted; the commit touches only the 3 scoped files.
- [x] nextest override present and only affects storage_growth: temporarily `period = "3s", terminate-after = 1` → in `cargo nextest run -p tinwave-db`, 5 storage_growth tests TIMEOUT; the other 64 (two short storage_growth tests under 3 s, all other tinwave-db binaries) passed. Real values (60s x 5) restored.
- [x] fmt, clippy -D warnings, tests, deny pass.

checks: fmt pass, clippy pass, tests 480 passed / 0 failed (6 skipped; nextest --workspace; and `cargo test -p tinwave-db --test storage_growth` 7/7), deny pass
files changed: crates/tinwave-db/tests/growth/sim.rs, crates/tinwave-db/tests/storage_growth.rs, .config/nextest.toml (new)
signals added/changed: none
ui: none
docs updated: module doc and `WRITER_CPU_BUDGET` doc in storage_growth.rs: measured figures (~6 s writer CPU debug, ~3 s release) replace "debug is as fast"; the check covers writer CPU work only; names the pragma test (`connection::tests::a_new_database_gets_every_pragma_of_database_md`) and the signal_writer timing tests for I/O and latency; notes checkpoint cost is not measured; points to the nextest backstop. docs/rules/testing.md untouched (out of scope).

details:
- Barrier wait: the writer exposes progress only through plain atomics and has no signalling hook; `writer.rs` is out of scope, so no Condvar — sleep with backoff. Benchmarked on Windows (Rust 1.92): `thread::sleep` floor ~0.55 ms for any request up to 200 us; `park_timeout` costs 15.6 ms even for 1 us (not used). `BARRIER_POLL_FIRST` = 50 us, `BARRIER_POLL_MAX` = 500 us, commented. 60 s hang assert and deterministic rows unchanged.
- `WRITER_CPU_BUDGET` = `if cfg!(debug_assertions) { 15 s } else { 8 s }`. Release measured 3.1/3.2/3.5/3.6 s (8 s = 2.2-2.6x); debug 5.8-6.3 s (15 s = 2.4-2.6x). Generator CPU printed, not asserted.
- nextest 0.9.150: `[[profile.default.overrides]] filter = 'binary(storage_growth)'`, `slow-timeout = { period = "60s", terminate-after = 5 }` → kill at 300 s (~20x idle, ~2.8x the worst measured 107 s). A comment says a future 5-year test in this binary (T-0029) must raise N. Custom profiles inherit it from `default`.

deviations from ticket:
- Idle wall clock +~3 s (release 11.3 → ~14.5 s, debug 14.2 → 16.9 s): ~0.55 ms Windows sleep granularity over ~11,400 barriers. Only a writer-side notification (Condvar in writer.rs) would avoid it; see proposal. The CPU budget doesn't depend on wall clock; the kill timeout has margin.
- "Condvar or park with timeout" became sleep backoff (the ticket also lists it), for the reasons above.

proposals:
- P: Writer-side wait/notify for committed events (`Condvar` or `wait_committed(target, timeout)` in `SignalWriter`) | why: the barrier could block exactly until the commit, cutting ~3 s of idle wall clock; helps any test waiting for the writer | size: S | evidence: crates/tinwave-db/src/writer.rs:205, tests/growth/sim.rs:1047
- P: Pointer in docs/rules/testing.md (storage growth section) to the per-profile writer CPU budget and the nextest backstop | why: the rule only mentions `--release` and not what the speed check covers | size: S | evidence: docs/rules/testing.md:56-69

questions for lead:
- None.

---
Lead (2026-11-03): sleep backoff and +3 s idle accepted. P-0152 (writer wait/notify) parked — a production API for a
test's convenience; revisit if a real consumer needs commit notifications. P-0153 (testing.md pointer) accepted → lead
applies on merge.
