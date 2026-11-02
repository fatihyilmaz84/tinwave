---
id: T-0031
title: Engine boundary follow-ups — resume at a track's exact end, positions with no current track, open-wait doc
type: bug
epic: E-01
status: in_progress
priority: P0
size: S
depends_on: []
scope:
  - crates/tinwave-playback/src/engine.rs
  - crates/tinwave-playback/src/reader.rs
  - crates/tinwave-playback/src/decoder.rs
  - crates/tinwave-playback/src/event.rs        # docs only
  - crates/tinwave-playback/src/output/windows.rs   # doc note only
  - crates/tinwave-playback/tests/
owner: tw-builder (round 1, same builder as T-0016/T-0017)
branch: t/T-0031-engine-boundary-followups
review_round: 0
created_by: lead, from T-0017 review-2 non-blocking 1–3
---

## Context
work/reports/T-0017/review-2.md. T-0024 maps `TrackLeft{Failed}` to neither Completed nor Skipped, so a completed
track must never be labelled Failed, and every event position must name an announced track.

## Scope
1. A `switch_device` (or seek) resuming at exactly a track's end (`frame == length`, reachable in the µs window after
   the T-0017 position clamp) must be "finished, advance": `TrackLeft{A, Finished}` + `TrackStarted{B, Advance}`, never
   `SeekFailed` / `TrackLeft{A, Failed}`.
2. With no current track (queue ended, appended track not yet heard), `position()` must not name the unheard track:
   return `None` (or the last announced position, documented) so `Paused`/`Resumed`/`VolumeChanged` never carry an item
   without a `TrackStarted`.
3. Doc note next to the drop-joins note on `WasapiOutput`: the post-claim wait in `open` is bounded only by the driver.

## Acceptance criteria
- [ ] Item 1: deterministic test (gated opener / fake output) at the exact boundary ⇒ Finished, not Failed; fails before.
- [ ] Item 2: test that events after queue end + append carry no unannounced item; fails before.
- [ ] Pairing property test (incl. `TINWAVE_PAIRING_SEEDS=256`), rt_alloc, concurrent stress (≥ 500 processes) green.
- [ ] fmt, clippy `-D warnings`, tests, `cargo deny check` pass; default run opens no device.

## Signals & storage
None (protects T-0024's Completed/Skipped mapping).
