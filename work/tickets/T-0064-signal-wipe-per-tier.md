---
id: T-0064
title: Wipe signal events per privacy tier
type: feature
epic: E-01
status: ready
priority: P2
size: S
depends_on: [T-0061]
scope:
  - crates/tinwave-db/src/signal_wipe.rs
  - crates/tinwave-db/src/wipe.rs
  - crates/tinwave-db/tests/signal_wipe.rs
  - docs/rules/privacy-and-signals.md   # wipe paragraph only
owner:
branch:
review_round: 0
created_by: from P-0257 (T-0061 build-1)
---

## Context
T-0060 wipes context fields per tier; T-0061 wipes signal events per category and everything. A per-tier wipe of signal
events (kind.rs `tier()`) is still missing, though privacy-and-signals.md promises it.

## Scope
- `SignalWiper::wipe_signal_tiers(&[Tier])`: the events of those tiers plus derived data by the T-0061 rule, in one
  transaction with the same single-scan cost; combined with the T-0060 context tier wipe so "wipe tier T2" is one call.

## Acceptance criteria
- [ ] Per tier: exactly that tier's events and derived data go — test per tier.
- [ ] Atomic, writer keeps working (reuse the contention helpers) — tests.
- [ ] fmt, clippy, nextest, deny pass.

## Added from T-0065 build-1 (P-0272)
- The reference-size contention tests assert a bound on the writer's wait (not only success within the 5 s busy
  timeout), chosen from measured quiet and parallel-load numbers.

- (P-0277) `signal_wipe::the_writer_thread_keeps_working_while_everything_is_wiped` failed once under parallel load
  (16.9 s); make it wait on conditions with budgets that hold under load.

- (P-0280) Under 32 heavy parallel processes the category-wipe contention test fails "database is locked" (3/48 on
  main too): assert against the measured wipe time or the writer's retry, not only the 5 s busy timeout.
- (P-0281) wipe.rs:301-303 + privacy doc: TRUNCATE busy usually means the writer holds the lock (PASSIVE already
  zeroed the main file; only the WAL keeps old frames); log and document both causes.

## Signals & storage
None.
