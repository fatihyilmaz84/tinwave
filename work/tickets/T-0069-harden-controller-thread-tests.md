---
id: T-0069
title: controller_thread tests wait on conditions, not on how far playback got
type: chore
epic: E-01
status: backlog
priority: P2
size: S
depends_on: [T-0057]
scope:
  - crates/tinwave-playback/tests/controller_thread.rs
owner:
branch:
review_round: 0
created_by: from P-0273 (T-0057 build-2)
---

## Context
Under heavy parallel load (40+ concurrent test processes) `dropping_the_handle_ends_the_session` and
`queue_edits_go_through_the_handle...` fail ~2-5 % of runs, also on main before T-0057: they assume how far playback
got (tests/controller_thread.rs:297, :343).

## Scope
- Replace the timing assumptions with bounded waits on observable conditions (events, positions), as T-0019/T-0057 did.

## Acceptance criteria
- [ ] 0 failures in 120 runs at 40 parallel copies alongside the other playback binaries — measured.
- [ ] The tests still fail when the behaviour they check is broken (mutation per test).
- [ ] fmt, clippy, nextest, deny pass.

## Signals & storage
None.
