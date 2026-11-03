---
id: T-0060
title: Wiping a context category also masks its fields in play.context_key
type: feature
epic: E-01
status: in_progress
priority: P1
size: S
depends_on: [T-0029]
scope:
  - crates/tinwave-db/
  - crates/tinwave-signals/src/context_key.rs   # a field-mask helper only
  - docs/rules/privacy-and-signals.md           # the wipe paragraph only
owner: tw-builder
branch: t/T-0060-wipe-masks-context-key
review_round: 1
created_by: from P-0240 (T-0029 review-1)
---

## Context
privacy-and-signals.md requires that the listener can wipe data per category / privacy tier. Since T-0029 every play
row keeps a `context_key` (ContextKey v1: daypart, weekday type, weather, device class, activity, place slot) for the
3-year warm tier. Today a wipe of, say, the place or weather category deletes the hot-tier context but leaves those
fields in the play rows.

## Scope
- A `ContextKey` helper that clears chosen fields to 0 (unknown), keeping the version rule (all-unknown key is raw 0).
- The existing category/tier wipe also rewrites `play.context_key` for the affected fields, in the same transaction
  as the rest of the wipe; monthly rollups carry no context and are untouched.
- Turning a category off (not wiping) stops new values only; existing rows change only on wipe.

## Acceptance criteria
- [ ] Wiping each T2/T3 context category zeroes exactly its ContextKey fields in every play row; other fields untouched — test.
- [ ] A key whose last known field is wiped becomes raw 0 — test.
- [ ] The wipe is atomic (a failure leaves play rows and hot-tier data unchanged) — test.
- [ ] fmt, clippy, nextest, deny pass; the storage growth test still passes.

## Signals & storage
No new signal; rows are updated in place.
