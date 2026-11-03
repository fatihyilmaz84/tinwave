---
id: T-0076
title: Rate-throttle clock owned by EngineConfig; tests advance it instead of sleeping
type: chore
epic: E-01
status: backlog
priority: P3
size: S
depends_on: [T-0063]
scope:
  - crates/tinwave-playback/src/engine.rs
  - crates/tinwave-playback/src/decoder.rs
  - crates/tinwave-playback/tests/rt_alloc.rs
  - crates/tinwave-playback/tests/controller_signals.rs
owner:
branch:
review_round: 0
created_by: from P-0306, P-0307 (T-0063 build-1)
---

## Context
T-0063 lets a rate flush through when 125 ms have passed on the monotonic clock or on `EngineConfig.clock` (the
event clock), because that was the only injection point in scope. rt_alloc.rs and controller_signals.rs still sleep
130 ms against a frozen manual clock.

## Scope
- Add a dedicated throttle clock to `EngineConfig` (monotonic in production, manual in tests); drop the hybrid.
- rt_alloc.rs:340 and the controller_signals.rs rate tests advance it instead of sleeping.
- Fix the `EngineConfig::clock` rustdoc (engine.rs:93).

## Acceptance criteria
- [ ] No 130 ms throttle sleep left in tinwave-playback tests.
- [ ] Throttle mutation (off) still fails the drag test.
- [ ] fmt, clippy, nextest, deny pass.

## Signals & storage
None.
