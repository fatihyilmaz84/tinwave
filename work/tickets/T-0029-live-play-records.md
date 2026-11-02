---
id: T-0029
title: Write play records live when a play ends (warm tier)
type: feature
epic: E-01
status: backlog
priority: P1
size: M
depends_on: [T-0006, T-0024]
scope:
  - crates/tinwave-db/
owner:
branch:
review_round: 0
created_by: from P-0071
---

## Context
database.md "Write path": play records are written live when a play ends, so the warm tier is always current and the
taste engine never scans raw events. T-0006 creates the `play` table and flushes at play end, but derives no rows.

## Scope
- Derive one `play` row per play from the play's events (started_ts, listened_ds, end_reason, end_frac, flags,
  peak_section, context_key) in the same transaction as the play-end batch.

## Acceptance criteria
- [ ] Each play end (Completed, Skipped*, Stopped, Quit) writes exactly one `play` row in the same transaction — test.
- [ ] A failed track (`TrackLeft{Failed}`) writes no play row or a distinct end_reason — decide and test.
- [ ] Growth test (T-0007) accounts for play rows.

## Signals & storage
Warm tier per storage-budget.md.

## Added from P-0111 (T-0024 review)
A play's Completed/Skipped can land in a later session than its PlayStarted (stale play after a 30-min gap). Pair a play start with its outcome per device (by seq order), never per session; test it.

## Added from P-0127 (T-0032 review)
In crates/tinwave-db/src/settings.rs test `the_mask_is_read_under_the_connection_lock`, send `go` before the in-scope asserts (or a drop guard) so a failure cannot hang the test; tidy the trigger SQL literal whitespace in tests/signal_settings.rs:452. Scope extends to those two spots.
