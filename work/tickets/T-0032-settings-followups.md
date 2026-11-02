---
id: T-0032
title: Signal settings follow-ups — clear the paused banner after a save, read-only producer view, lock invariant test
type: chore
epic: E-01
status: done
priority: P2
size: S
depends_on: []
scope:
  - crates/tinwave-db/src/settings.rs
  - crates/tinwave-db/tests/signal_settings.rs
  - crates/tinwave-signals/src/settings.rs
owner: tw-builder (round 1)
branch: t/T-0032-settings-followups
review_round: 1
created_by: from T-0021 review-1 non-blocking 1, 3, 4
---

## Context
work/reports/T-0021/review-1.md.

## Scope
1. `load_problem()` clears in-session after the first successful save (no stale "collection is paused" banner).
2. Producers get a read-only view of `SignalSettings` (or documented "toggle only through PersistentSettings"), so a
   toggle can't change the mask without persisting it.
3. Deterministic test pinning "the upsert reads the mask under the connection lock" (hook between the atomic update
   and the save).

## Acceptance criteria
- [ ] Each item tested (item 3 fails with the mask read before the lock).
- [ ] fmt, clippy `-D warnings`, tests, `cargo deny check` pass.

## Signals & storage
None.

## Outcome
2026-11-02: approved round 1, merged 48b97ad. Item 2 as documented contract (P-0126 parked). Review nits (scoped-thread test can hang if an in-scope assert fails; SQL whitespace) → P-0127 folded into T-0029.
