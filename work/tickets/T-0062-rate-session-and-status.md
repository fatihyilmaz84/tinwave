---
id: T-0062
title: Derive RateSession per play and expose the applied rate in PlaybackStatus
type: feature
epic: E-01
status: done
priority: P1
size: S
depends_on: [T-0027]
scope:
  - crates/tinwave-playback/src/controller/
  - crates/tinwave-playback/tests/controller_signals.rs
  - docs/guides/signal-catalog.md   # RateSession row only
owner: tw-builder
branch: t/T-0062-rate-session-and-status
review_round: 1
merge: 30cfa03
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

## Lead decisions on build-1 (2026-11-03)
- Delivery via `take_rate_sessions()` (derived kinds can't go on the bus) is accepted as a stopgap; the real path is
  T-0070. The `lib.rs` re-export is accepted.
- A play spent entirely at 1.0 yields 1.0 ("not rushed" vs "not measured"); a play straddling a Speed toggle yields
  none — confirmed.

## Signals & storage
Emits the catalogued derived `RateSession` (≤ 1 per play).
