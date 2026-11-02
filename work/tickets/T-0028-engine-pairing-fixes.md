---
id: T-0028
title: Fix engine leave-pairing edge cases and the load-dependent test hang; add a pairing property test
type: bug
epic: E-01
status: done
priority: P0
size: S
depends_on: []
scope:
  - crates/tinwave-playback/
owner: tw-builder (round 1, same builder as T-0016)
branch: t/T-0028-engine-pairing-fixes
review_round: 1
created_by: lead (PO decision D-0005), P-0059
---

## Context
T-0016 was merged with known issues (D-0005). Review: work/reports/T-0016/review-2.md (blocking 1-3, non-blocking).
Every `TrackStarted` must have exactly one `TrackLeft`; the signal layer (T-0024) turns violations into phantom
"completed" plays.

## Scope
1. Seek after `QueueEnded` must not produce a second `TrackLeft` (reject with `NothingLoaded`, or restart with a
   `TrackStarted{Requested}` before `Seeked`; choose, document, test).
2. A failed `switch_device` must not produce a duplicate `TrackStarted` (record the sent command on the error path).
3. `a_track_failing_mid_decode_is_left_as_failed_not_finished` (failures.rs:229) and sibling loops: when the ring is
   empty, render+poll in a bounded loop like `Harness::finish` instead of waiting for buffered frames.
4. Property test (P-0059): random sequences of load / append / seek / stop / switch_device / fail-open / fail-mid-decode
   driven through the engine with `ManualOutput`, checking `assert_paired` and monotonic position after each step;
   deterministic seed; enough cases to have caught both blocking bugs (show it fails on the pre-fix code).
5. Non-blocking from review-2: per-renderer "has seen data since open" flag instead of resetting the shared `primed`
   (closes the remaining spurious-Underrun window).

## Acceptance criteria
- [ ] The reviewer's two repro scenarios are tests and pass; they fail on the pre-fix code.
- [ ] Property test passes and is shown to catch both bugs on the pre-fix code.
- [ ] Concurrent stress run (playback test binaries × ≥ 6 copies, ≥ 500 processes): 0 failures.
- [ ] Zero callback allocations still hold; fmt, clippy `-D warnings`, tests, `cargo deny check` pass.

## Signals & storage
None (engine events only).

## Merged
2026-11-02, merge commit c0bfd2a (branch head 517b533). Review: review-1 (APPROVE). Gates green on main (226 tests). Non-blocking findings -> T-0030.
