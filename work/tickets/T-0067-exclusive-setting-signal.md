---
id: T-0067
title: ExclusiveModeToggled from the listener's setting, not from stream modes
type: feature
epic: E-01
status: in_review
priority: P1
size: S
depends_on: [T-0057]
scope:
  - crates/tinwave-playback/src/controller/
  - crates/tinwave-playback/src/output/mod.rs     # exposing the exclusive control flag only
  - crates/tinwave-playback/tests/controller_exclusive.rs
  - crates/tinwave-playback/tests/controller_signals.rs
owner: tw-builder
branch: t/T-0067-exclusive-setting-signal
review_round: 1
created_by: from P-0267 (T-0057 build-1b)
---

## Context
T-0057: automatic fallbacks emit no `ExclusiveModeToggled` (lead ruling). Because the mapper derives the toggle from
stream modes, a listener turning exclusive off while already in fallback Shared emits nothing — a preference change
without its signal.

## Scope
- The controller sees the listener's exclusive setting (the control flag) and emits `ExclusiveModeToggled{on}` when the
  setting changes, coalesced as today, whatever the stream does; stream-mode changes alone (fallbacks, reopens) never emit.

## Acceptance criteria
- [ ] Toggle off/on during fallback Shared emits exactly one signal each — test.
- [ ] Fallbacks and rate reopens emit nothing; the T-0057 tests still pass — tests.
- [ ] fmt, clippy, nextest, deny pass.

## Added from T-0057 review-1
- mapper.rs:984-991: the fallback branch sets `reported_mode = info.mode` unconditionally and can erase a deferred
  listener toggle (listener turns exclusive on outside a session, then an append reopens at a refused rate). Cover it.

## Signals & storage
No new kind; source of an existing signal moves from stream mode to the setting.
