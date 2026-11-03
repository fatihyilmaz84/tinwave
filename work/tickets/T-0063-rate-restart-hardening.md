---
id: T-0063
title: Rate restart hardening — no flush backoff, relative click test, injectable throttle clock, restart failure reports
type: chore
epic: E-01
status: in_review
priority: P2
size: S
depends_on: [T-0027]
scope:
  - crates/tinwave-playback/src/decoder.rs
  - crates/tinwave-playback/tests/rate.rs
owner: tw-builder
branch: t/T-0063-rate-restart-hardening
review_round: 1
created_by: from P-0252, P-0253 (T-0027 review-1) and the T-0027 build-1 throttle-clock question
---

## Context
work/reports/T-0027/review-1.md non-blocking findings.

## Scope
- While awaiting a rate restart's flush ack, poll at the minimum interval instead of backing off to 16 ms (P-0253).
- tests/rate.rs: the no-click test compares the max step within ~30 ms of each restart with the same recording's steady
  max (P-0252), replacing the 1.03x-analytic bound.
- Inject the throttle clock (decoder.rs uses std `Instant`) so rate.rs needs no 130 ms sleeps and is deterministic.
- `restart_audible` re-reading a track that fails again: report the failure once per item, and a failed reopen leaves
  the item as Failed, not Finished.

- (T-0066 build-1) `rate::a_drag_costs_a_few_flushes...` and `rate::no_click_at_a_rate_step...` failed once under 100 % CPU
  load (one flush for 48 steps over 492 ms): the injectable throttle clock must make them load-independent.

## Acceptance criteria
- [ ] A 2 ms fade is caught by the relative click test; a 5 ms fade passes — test.
- [ ] Rate restart reaches the first new audio without backoff delay (simulated device) — test.
- [ ] rate.rs has no real sleeps for the throttle and runs faster; stable under 16 parallel copies.
- [ ] A track failing again on restart reports one TrackFailed and leaves as Failed — test.
- [ ] fmt, clippy, nextest, deny pass.

## Signals & storage
None.
