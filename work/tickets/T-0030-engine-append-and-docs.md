---
id: T-0030
title: Engine follow-ups — append-after-end buffering, third-fix regression test, append+switch window, event docs
type: bug
epic: E-01
status: in_review
priority: P0
size: S
depends_on: []
scope:
  - crates/tinwave-playback/
owner: tw-builder (round 1, same builder as T-0016/T-0028)
branch: t/T-0030-engine-append-and-docs
review_round: 1
created_by: lead, from T-0028 review-1 non-blocking findings
---

## Context
work/reports/T-0028/review-1.md. T-0024 (controller) relies on: one TrackLeft per TrackStarted, no Seeked without a
current track, and "a stalled first read is buffering, not an Underrun".

## Scope
1. Append after the queue ended: a stall while the decoder opens the new file must not count as Underruns (probe:
   finish A, append B, 200 fast callbacks ⇒ 198 Underrun today). Decoder clears `eos` only with the first push after
   revival, or a stream-epoch atomic the renderer compares to reset `seen_data` — builder's choice, documented.
2. Directed regression test for the third T-0028 fix (decoder keeps `st.cur` on `Reconfigure{item: None}`): queue
   ended → append → `switch_device` before the appended track is heard ⇒ appended track plays, pairing holds. Must
   fail with the fix reverted.
3. Append + switch window: if the queue ended and a short appended track is fully decoded but not yet heard, a
   `switch_device` must not silently drop it. Fix and test.
4. event.rs docs: `Seeked` only occurs with a current track; `Underrun` only after audio has flowed since the last
   open or flush.

## Acceptance criteria
- [ ] Each item has a test that fails before its fix (state how you checked).
- [ ] Property test and concurrent stress (≥ 500 processes) still 0 failures; zero callback allocations.
- [ ] fmt, clippy `-D warnings`, tests, `cargo deny check` pass.

## Signals & storage
None.
