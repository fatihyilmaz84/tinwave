---
id: T-0079
title: image-stress teardown under extreme load — slow or hung; then HANG_AFTER
type: chore
epic: E-01
status: backlog
priority: P3
size: S
depends_on: [T-0071]
scope:
  - apps/tinwave-desktop/tests/image_provider.rs
  - apps/tinwave-desktop/tools/stress/
owner:
branch:
review_round: 0
created_by: from P-0314, P-0315 (T-0071 build-1)
---

## Context
Under ~10x machine slowdown, 14 of 100 cancel_storm runs printed STRESS_DONE but not STRESS_TEARDOWN_COMPLETE within
`HANG_AFTER` (60 s). No run was seen stuck for 300 s, but no stuck run was inspected. ADR 0008 records an older 2/300
hang class.

## Scope
- Capture what a slow teardown is doing (timestamps between STRESS_DONE, `drop(shell)` steps and
  STRESS_TEARDOWN_COMPLETE; a stack dump if one passes, say, 120 s) under the T-0071 heavy-load recipe.
- If it is only slow: raise `HANG_AFTER` (~180 s) with the measured reason in a comment.
- If it is a real hang: stop and report with the evidence (it becomes a provider/bridge ticket).

## Acceptance criteria
- [ ] The handoff states slow or hung, with timings from at least 20 slow teardowns.
- [ ] If slow: HANG_AFTER raised, and a mutation that hangs teardown forever is still caught.
- [ ] fmt, clippy, nextest, deny, `cargo build --workspace` pass.

## Signals & storage
None.
