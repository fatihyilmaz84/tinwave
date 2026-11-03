---
id: T-0061
title: Wipe signal history per signal category and "wipe everything"
type: feature
epic: E-01
status: done
priority: P1
size: M
depends_on: [T-0060]
scope:
  - crates/tinwave-db/
  - docs/rules/privacy-and-signals.md   # the wipe paragraph only
owner: tw-builder
branch: t/T-0061-signal-wipe-per-category-and-all
review_round: 1
merge: fb5b252
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

## Added from T-0060 review-2
- tests/context_wipe.rs `a_write_batch_during_a_wipe_of_the_reference_table_is_not_starved`: assert (or record) that the
  probe observed the wipe's lock at least once, so a fast machine cannot silently skip the contention. Reuse the same
  contention pattern for the new wipes.

## Lead decisions on build-1 (2026-11-03)
- Q1: wiping Queue & library deletes all play rows and rollups — **confirmed** (an outcome with no recording or time
  carries nothing). The Signal Center must say so before the listener confirms (P-0244).
- Q2: every category wipe resets `rollup_monthly.score_sum` — **confirmed**: it blends all categories, so it is derived
  from each (the T-0060 rule). Nothing writes rollups yet.
- Q3: the seek/skip/replay wipe keeps the row with `end_reason = Unknown` (6) — **confirmed**; readers treat 6 as "no
  information", never a skip (documented in storage-budget.md on merge, P-0255).
- Q4: unknown kind codes are removed only by wipe everything — **confirmed**.
- Accepted outside the ticket text: `secure_delete=ON` on the wiper connection; the `data_version` cache check in
  `SignalStore::write_batch`.
- P-0256 (seq high-water across wipe everything) is a requirement for E-05 sync.

## Signals & storage
No new signal.
