---
id: T-0062
title: Derive RateSession per play and expose the applied rate in PlaybackStatus
type: feature
epic: E-01
status: ready
priority: P1
size: S
depends_on: [T-0027]
scope:
  - crates/tinwave-playback/src/controller/
  - crates/tinwave-playback/tests/controller_signals.rs
  - docs/guides/signal-catalog.md   # RateSession row only
owner:
branch:
review_round: 0
created_by: from P-0249, P-0250 (T-0027 build-1)
---

## Context
T-0027 emits `RateChanged` and gives the controller `rate_stats()` / `RateStats::time_weighted_rate_since`, but the
derived per-play `RateSession` (signal catalog §2, `time_weighted_rate`) is not emitted, and `PlaybackStatus` has no rate.

## Scope
- Snapshot `rate_stats()` at PlayStarted and at the play's leave; emit `RateSession{time_weighted_rate}` once per play,
  only when the Speed category is on and the play was heard; none for a play entirely at 1.0 if the catalog says so
  (decide from the catalog row; record the decision).
- `PlaybackStatus.rate`: the engine's applied rate.

## Acceptance criteria
- [ ] One RateSession per heard play with the time-weighted rate within 0.1 % of the schedule — test.
- [ ] No RateSession with Speed off, or for an unheard/failed play — test.
- [ ] PlaybackStatus reports the applied rate after a throttled change (latest value wins) — test.
- [ ] Catalog row, storage estimate and Signal Center entry consistent; growth test passes.

## Signals & storage
Emits the catalogued derived `RateSession` (≤ 1 per play).
