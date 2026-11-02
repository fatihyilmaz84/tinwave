---
id: T-0025
title: Short fade on seek and track-skip flush (no clicks)
type: feature
epic: E-01
status: ready
priority: P2
size: S
depends_on: [T-0016]
scope:
  - crates/tinwave-playback/src/renderer.rs
  - crates/tinwave-playback/src/decoder.rs   # P-0081 only
  - crates/tinwave-playback/tests/append.rs   # P-0081 only
owner:
branch:
review_round: 0
created_by: from P-0043
---

## Context
T-0016 ramps pause and volume, but a seek/skip flush cuts audio abruptly and can click in loud passages.

## Also (P-0081, T-0030 review)
- Leave `buffering` set in `finish_queue` (closes a one-callback spurious Underrun).
- Make `switch_while_an_appended_track_is_still_opening_does_not_drop_it` fail directly (B never opened / no audio
  after the gate) instead of by a 16 s timeout.

## Acceptance criteria
- [ ] Seek/skip output has no step discontinuity above a threshold — test on a loud sine.
- [ ] Seek still lands sample-exact after the fade-in; zero callback allocations.
