---
id: T-0048
title: Growth-test harness barrier waits without spinning a core
type: chore
epic: E-00
status: ready
priority: P2
size: S
depends_on: [T-0047]
scope:
  - crates/tinwave-db/tests/growth/
owner:
branch:
review_round: 0
created_by: from P-0144
---

## Context
T-0047: the storage-growth harness barrier (crates/tinwave-db/tests/growth/sim.rs ~1063) waits for the writer with a
`yield_now` loop. Under load the producer thread burned 88.7 s of a 95.8 s run, adding a busy core to every gate run.

## Scope
- Replace the spin with a blocking wait (Condvar / park with timeout, or a short sleep backoff); keep the existing 60 s
  hang timeout and the deterministic row output.

## Acceptance criteria
- [ ] Producer-thread CPU during the 90-day run drops to a small fraction of wall clock (numbers in the handoff).
- [ ] `the_same_seed_gives_the_same_sizes` and `the_rows_are_the_same_when_the_producer_is_preempted_after_a_pause`
      still pass; sizes unchanged.
- [ ] fmt, clippy -D warnings, tests, deny pass.

## Signals & storage
None (test only).
