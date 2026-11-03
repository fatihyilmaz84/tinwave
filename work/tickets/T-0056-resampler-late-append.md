---
id: T-0056
title: Keep the resampler continuous when a same-rate track is appended after the decoder read the last one
type: feature
epic: E-01
status: backlog
priority: P2
size: M
depends_on: [T-0019]
scope:
  - crates/tinwave-playback/
owner:
branch:
review_round: 0
created_by: from P-0221 (T-0019 build-1)
---

## Context
T-0019: when a track is appended after the decoder already read the last queued track, the end-of-queue flush has
emitted the resampler tail, so the appended track starts a fresh resampler (revive path) — an ~80-frame filter
transient at a join that should be gapless (live albums added a track at a time).

## Scope
- Hold the end-of-queue flush until the ring would actually underrun (or re-prime the new resampler with the last input
  frames), so a late same-rate append continues seamlessly; an underrun still ends cleanly.

## Acceptance criteria
- [ ] Late append of a same-rate track after the decoder hit the end: the join is bit-close to the uncut sweep (same
      tolerance as the continuous-boundary test) — test; revert-tested.
- [ ] A real end of queue still drains the tail and reports QueueEnded once; rt_alloc green.
- [ ] fmt, clippy -D warnings, tests, deny pass.

## Signals & storage
None.
