---
id: T-0048
title: Growth-test harness: no spinning barrier, per-profile CPU budget, hard hang backstop
type: chore
epic: E-00
status: in_review
priority: P2
size: S
depends_on: [T-0047]
scope:
  - crates/tinwave-db/tests/growth/
  - crates/tinwave-db/tests/storage_growth.rs
  - .config/nextest.toml          # new: per-test terminate-after override for storage_growth only
owner: tw-builder
branch: t/T-0048-growth-barrier-no-spin
review_round: 1
created_by: from P-0144, P-0148, T-0047 review-1
---

## Context
T-0047: the storage-growth harness barrier (crates/tinwave-db/tests/growth/sim.rs ~1063) waits for the writer with a
`yield_now` loop. Under load the producer thread burned 88.7 s of a 95.8 s run, adding a busy core to every gate run.

## Scope
- Replace the spin with a blocking wait (Condvar / park with timeout, or a short sleep backoff); keep the existing 60 s
  hang timeout and the deterministic row output.

- From T-0047 review-1: `WRITER_CPU_BUDGET` per build profile (`cfg!(debug_assertions)`: 15 s debug, ~8 s release,
  each ~2.4x the measured value — measure release first); fix the module doc's "debug is as fast" sentence; say in the
  doc that the check measures writer CPU work only (I/O and latency regressions are covered by the pragma test and the
  signal_writer timing tests).
- A hard hang backstop: `.config/nextest.toml` override for the storage_growth binary with
  `slow-timeout = { period = "60s", terminate-after = N }` (N so an idle-machine run never trips it; justify N).

## Acceptance criteria
- [ ] Producer-thread CPU during the 90-day run drops to a small fraction of wall clock (numbers in the handoff).
- [ ] `the_same_seed_gives_the_same_sizes` and `the_rows_are_the_same_when_the_producer_is_preempted_after_a_pause`
      still pass; sizes unchanged.
- [ ] `cargo test --release -p tinwave-db --test storage_growth`: a 3x writer slowdown (temporary mutation) fails.
- [ ] nextest override present and only affects storage_growth.
- [ ] fmt, clippy -D warnings, tests, deny pass.

## Signals & storage
None (test only).
