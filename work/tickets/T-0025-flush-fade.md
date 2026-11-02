---
id: T-0025
title: Short fade on seek and track-skip flush (no clicks)
type: feature
epic: E-01
status: backlog
priority: P2
size: S
depends_on: [T-0016]
scope:
  - crates/tinwave-playback/src/renderer.rs
owner:
branch:
review_round: 0
created_by: from P-0043
---

## Context
T-0016 ramps pause and volume, but a seek/skip flush cuts audio abruptly and can click in loud passages.

## Acceptance criteria
- [ ] Seek/skip output has no step discontinuity above a threshold — test on a loud sine.
- [ ] Seek still lands sample-exact after the fade-in; zero callback allocations.
