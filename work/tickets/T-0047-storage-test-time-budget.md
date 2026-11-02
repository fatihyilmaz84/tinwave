---
id: T-0047
title: storage_growth speed check must not fail just because the machine is busy
type: chore
epic: E-00
status: in_progress
priority: P1
size: S
depends_on: []
scope:
  - crates/tinwave-db/tests/storage_growth.rs
  - docs/rules/storage-budget.md     # only if the speed rule's wording changes
owner: tw-builder
branch: t/T-0047-storage-test-time-budget
review_round: 1
created_by: from P-0138
---

## Context
`ninety_days_of_a_heavy_listener_fit_the_hot_window_and_the_five_year_budget` (storage_growth.rs:288) asserts the
90-day simulation finishes in < 60 s of wall clock. Idle it takes ~32 s; on 2026-11-02 it took 70 s and 97 s while other
agents ran test suites in parallel (CPU 100 %), failing the main gate although every size was within budget.
The speed check exists to catch an accidental slowdown of the writer path, not machine load.

## Scope
- Replace the wall-clock assertion with a load-independent measure (e.g. the test thread's and writer thread's CPU
  time, or per-transaction cost), or split the speed check into its own clearly named test with a budget derived from
  a calibration loop run in the same process. No change to the size assertions.
- Keep the printed report (sizes, extrapolation) unchanged.

## Acceptance criteria
- [ ] The test passes with the machine saturated (e.g. run alongside 2 copies of the full workspace nextest) — numbers
      in the handoff.
- [ ] A deliberate 3× slowdown of the writer path (temporary mutation, reverted) still fails the speed check.
- [ ] fmt, clippy -D warnings, tests, deny pass.

## Signals & storage
None (test only).
