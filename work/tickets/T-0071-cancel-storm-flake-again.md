---
id: T-0071
title: cancel_storm image-provider test flaked again ("nothing was cancelled")
type: fix
epic: E-01
status: in_review
priority: P2
size: S
depends_on: []
scope:
  - apps/tinwave-desktop/src/app/   # image provider only
  - apps/tinwave-desktop/tests/image_provider.rs
  - apps/tinwave-desktop/tools/stress/   # added by the lead in round 1 (the flaky timer lives in Storm.qml)
owner: tw-builder
branch: t/T-0071-cancel-storm-flake-again
review_round: 1
created_by: from P-0283 (T-0065 review-1)
---

## Context
T-0049 (`in_flight_at_quit`) was meant to fix `image_provider::cancel_storm_with_requests_in_flight_then_quit_exits_cleanly`.
It failed once more in a full-workspace run on 2026-11-03 ("nothing was cancelled") and passed on rerun.

## Scope
- Find why the cancel storm can complete without any cancellation under load, and make the test wait on a condition
  that proves requests were in flight at quit (or make the assertion match what the test controls).

## Acceptance criteria
- [ ] 0 failures in 100 runs alongside a loaded full-workspace run — measured.
- [ ] The test still fails when cancellation at quit is broken (mutation).
- [ ] ADR 0008 image-provider rules still hold; fmt, clippy, nextest, deny pass.

## Signals & storage
None.
