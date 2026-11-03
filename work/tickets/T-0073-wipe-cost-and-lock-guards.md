---
id: T-0073
title: Wipe tests pin the statement count and catch a lock left open; SECTIONED check set-equal
type: chore
epic: E-01
status: backlog
priority: P2
size: S
depends_on: [T-0064]
scope:
  - crates/tinwave-db/tests/signal_wipe.rs
  - crates/tinwave-db/tests/context_wipe.rs
  - crates/tinwave-db/tests/common/mod.rs
  - crates/tinwave-db/src/signal_wipe.rs   # SECTIONED check only
  - docs/rules/privacy-and-signals.md      # the "idle interval" sentence only
owner:
branch:
review_round: 0
created_by: from P-0295, P-0297 (T-0064 review-1)
---

## Context
work/reports/T-0064/review-1.md non-blocking findings: after T-0064 the contention tests prove liveness and no lost batch
but do not bound the wipe's own cost, and the after-wipe bound can almost never trip.

## Scope
- Count the SQL statements a wipe runs (rusqlite `trace`/`profile` on the wiper connection) and pin them per wipe kind,
  independent of row count.
- The reference contention test sends a second batch after the wipe returns (catches a lock the wiper left open).
- signal_wipe.rs:540-547: the SECTIONED check is set-equal, not "contains".
- privacy-and-signals.md:80: the "at most its idle interval (60 s) later" sentence does not hold while a reader stays open.

## Acceptance criteria
- [ ] Mutations: a per-row loop in a wipe, and a lock left open after a wipe, each fail a test.
- [ ] fmt, clippy, nextest, deny pass.

## Signals & storage
None.
