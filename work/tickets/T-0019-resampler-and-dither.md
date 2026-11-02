---
id: T-0019
title: Resampler (rubato) in the decoder thread and exclusive-mode float-to-int dither policy
type: feature
epic: E-01
status: backlog
priority: P1
size: M
depends_on: [T-0016]
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
- TPDF dither for float -> i16/i24 in exclusive mode; none for shared (f32). T-0017 convert.rs is round-to-nearest
  today; exclusive I16 (last resort) with gain != 1 or 24-bit material is the case that needs it (T-0017 review).

## Notes
- ADR 0006: time-stretch runs at the file rate before the resampler; check 96 kHz and mono CPU (P-0055).

## Acceptance criteria
- [ ] 44.1 kHz pair on a 48 kHz endpoint stays gapless (<= 1 frame) — test.
- [ ] Rate change between tracks plays without glitch and without callback allocation — test.
- [ ] Dither is applied only for integer outputs — unit tests.
- [ ] fmt, clippy `-D warnings`, tests, `cargo deny check` pass.

## Signals & storage
None.
