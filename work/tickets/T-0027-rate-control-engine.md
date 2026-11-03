---
id: T-0027
title: Pitch-preserving rate control in the playback engine (bypass at 1.0, flush-and-restart, rate counters)
type: feature
epic: E-01
status: ready
priority: P1
size: M
depends_on: [T-0026]
scope:
  - crates/tinwave-playback/
owner:
branch:
review_round: 0
created_by: from P-0051
---

## Context
ADR 0006 and spike T-0009 (`spikes/T-0009-time-stretch/src/engine_sim.rs`). Signal catalog §2: `RateChanged` (per
gesture) and derived `RateSession` (time-weighted rate) need the engine to report the effective rate per block.

## Scope
- Stretch stage in the decoder thread, bypass at exactly 1.0×; segment queue; renderer publishes `eff_rate`,
  `wall_frames`, `src_micro` per block; rate change = flush-and-restart at the audible source position, 5 ms fades;
  at most one flush per 100–150 ms during a drag, final value always applied.
- PlaybackEvents for rate changes (the controller T-0024 turns them into `RateChanged`).

## Acceptance criteria
- [ ] Zero callback allocations across rate changes — rt_alloc test.
- [ ] Position accuracy through 4 rate changes ≤ 3 ms mean on a click train — test.
- [ ] Time-weighted rate over a 1.0→1.25→0.75→1.5→1.0 schedule within 0.1 % — test.
- [ ] No click at a rate step (largest sample step ≤ the steady signal's) — test.
- [ ] Bypass at 1.0 is bit-exact with the un-stretched path — test.

## Signals & storage
Enables `RateChanged` / `RateSession`; no new kinds.

## Added from P-0206 (T-0026 build-1)
- Turn the ADR 0006 decision-3 rate arithmetic (fixed output block, `floor(out*rate + carry)` input) from the
  `drive()` helper in crates/tinwave-playback/tests/stretch.rs into a tested production helper.
- (P-0209, T-0026 review-1) Tighten tests/stretch.rs onset check first: a source with leading silence
  (< 2 x blockSamples = 10584 frames, so the silence bypass does not kick in) and the onset within about ±128 frames
  of `S/rate` (two-sided), so over- and under-discarding the ADR 0006 lag both fail.
