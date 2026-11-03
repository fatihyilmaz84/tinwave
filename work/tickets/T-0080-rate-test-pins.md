---
id: T-0080
title: Pin the flush-poll ack reset; record the rate restart index before new audio plays
type: chore
epic: E-01
status: backlog
priority: P3
size: S
depends_on: [T-0063]
scope:
  - crates/tinwave-playback/src/decoder.rs   # tests module only
  - crates/tinwave-playback/tests/rate.rs
owner:
branch:
review_round: 0
created_by: from P-0320, P-0321 (T-0063 review-2)
---

## Context
T-0063 review-2 found that removing the ack reset of the flush poll state (`flush_wait = AckPoll::new()`, decoder.rs
~375) passes every test, though it is what keeps a rate restart on fast polling after a command's backoff. It also found
that `Rec::step` records the restart index after a callback that may already have played new-stream audio, so under
load the index is one 256-frame callback late (harmless for coverage, but it makes the click margin load-dependent).

## Scope
- A unit or loop-level test: a command flush, then a rate flush (or two rate flushes in a row), asserts the second
  starts at FLUSH_WAIT_MIN with its full fast window.
- Record the restart index from the first new-stream frame, not after the callback; the 2 ms mutation still fails 3/3.

## Acceptance criteria
- [ ] Removing the ack reset fails the new test.
- [ ] Click test margin is the same idle and under 16 burners (report both), 2 ms fade still caught.
- [ ] fmt, clippy, nextest, deny, `cargo build --workspace` pass.

## Signals & storage
None.
