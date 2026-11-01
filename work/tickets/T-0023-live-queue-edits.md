---
id: T-0023
title: Live queue edits in the playback engine (play next, reorder, remove) without a flush
type: feature
epic: E-01
status: backlog
priority: P1
size: M
depends_on: [T-0016]
scope:
  - crates/tinwave-playback/
owner:
branch:
review_round: 0
created_by: from P-0041
---

## Context
T-0016's engine has `load` (flushes) and `append` only. Queue edits during playback (spec §7; catalog §4
`QueuedNext/Later`) must not interrupt the playing track or break gapless hand-off.

## Scope
- Decoder-side replace of the items after the head, with a stale-head check (the edit applies only if the head is
  still the item the UI saw).

## Acceptance criteria
- [ ] Play-next / reorder / remove while playing never interrupts the current track — test.
- [ ] Gapless hand-off into an edited next item stays 0-frame — test.
- [ ] A stale edit (head changed) is rejected with a visible error, not applied to the wrong item — test.
- [ ] Zero callback allocations still hold (rt_alloc test).

## Signals & storage
Inputs for `QueuedNext`/`QueuedLater`/`QueueReordered` wiring (T-0024).
