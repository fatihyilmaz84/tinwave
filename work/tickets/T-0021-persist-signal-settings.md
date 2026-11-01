---
id: T-0021
title: Persist Signal Center toggles (SignalSettings) across restarts
type: feature
epic: E-01
status: backlog
priority: P1
size: S
depends_on: [T-0006]
scope:
  - crates/tinwave-signals/src/settings.rs
  - crates/tinwave-db/
owner:
branch:
review_round: 0
created_by: from P-0031
---

## Context
T-0005's `SignalSettings` is an in-memory atomic mask (7 categories × T0–T3). docs/rules/privacy-and-signals.md:
toggles are checked at the source and must survive restarts; defaults T0, T1 on, T2, T3 off.

## Scope
- A settings row (or small table) in signals.db; load at startup before the bus accepts events; save on change.
- Turning a category off is effective immediately and stays off after restart.

## Acceptance criteria
- [ ] Toggle off → restart → still off, and zero events of that category constructed — integration test.
- [ ] First run uses the documented defaults — test.
- [ ] fmt, clippy `-D warnings`, tests, `cargo deny check` pass.

## Signals & storage
None (toggle changes write settings, not signals — design-system.md `TwSignalRow`).
