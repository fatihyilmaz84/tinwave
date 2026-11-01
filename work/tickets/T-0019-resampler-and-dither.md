---
id: T-0019
title: Resampler (rubato) in the decoder thread and exclusive-mode float-to-int dither policy
type: feature
epic: E-01
status: blocked
priority: P1
size: M
depends_on: [T-0016, D-0003]
scope:
  - crates/tinwave-playback/
owner:
branch:
review_round: 0
created_by: from P-0025
---

## Context
T-0003: rubato Fft 44.1 -> 48 kHz is about 2000x realtime; one continuous resampler across same-rate tracks avoids
boundary phase error; exclusive mode needs integer output (the spike truncates).

## Scope
- Resample in the decoder thread (never in the callback); continuous across same-rate tracks; flush on rate change.
- TPDF dither for float -> i16/i24 in exclusive mode; none for shared (f32).

## Acceptance criteria
- [ ] 44.1 kHz pair on a 48 kHz endpoint stays gapless (<= 1 frame) — test.
- [ ] Rate change between tracks plays without glitch and without callback allocation — test.
- [ ] Dither is applied only for integer outputs — unit tests.
- [ ] fmt, clippy `-D warnings`, tests, `cargo deny check` pass.

## Signals & storage
None.
