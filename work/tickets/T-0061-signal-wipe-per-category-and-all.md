---
id: T-0061
title: Wipe signal history per signal category and "wipe everything"
type: feature
epic: E-01
status: backlog
priority: P1
size: M
depends_on: [T-0060]
scope:
  - crates/tinwave-db/
  - docs/rules/privacy-and-signals.md   # the wipe paragraph only
owner:
branch:
review_round: 0
created_by: from P-0246 (T-0060 build-1)
---

## Context
privacy-and-signals.md promises the listener can wipe collected data per category, per tier, and everything. T-0060 adds
`SignalWiper` for the context fields of play rows and snapshot keys only. Deleting a signal category's events, and
"wipe everything" (signal events, counters, play rows, rollups, snapshots; `taste.db` once it exists), is missing.

## Scope
- Extend `SignalWiper`: wipe one signal category (its `signal_event` rows and counters, and what is derived from them in
  play rows/rollups, as the catalog's derivation rules say), and wipe everything.
- Each wipe is one transaction; the writer thread keeps working (busy timeout, no lost batch).
- Define what "derived from" means per category in privacy-and-signals.md (e.g. wiping Seek/skip clears play-row flags
  and end reasons? or deletes the rows?) — the builder proposes, the lead decides before merge.

## Acceptance criteria
- [ ] Per-category wipe deletes exactly that category's events and derived data — test per category.
- [ ] Wipe everything leaves empty signal tables and a working writer — test, including a batch in flight.
- [ ] Atomic under failure injection — test.
- [ ] fmt, clippy, nextest, deny; storage growth test passes.

## Signals & storage
No new signal.
