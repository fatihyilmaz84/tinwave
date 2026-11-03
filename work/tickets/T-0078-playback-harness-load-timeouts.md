---
id: T-0078
title: Playback test harness waits that survive a loaded machine
type: chore
epic: E-01
status: backlog
priority: P3
size: S
depends_on: [T-0063]
scope:
  - crates/tinwave-playback/tests/common/
owner:
branch:
review_round: 0
created_by: from P-0308 (T-0063 build-1)
---

## Context
`settle`, `wait_buffered` and `expect_edit_applied` give up after about 3 s of 100 µs sleeps; with other builds
running, correct tests fail there (main's rate binary too).

## Scope
- Make the waits progress-based (give up only when nothing advanced for a window) or raise them with a stated reason;
  keep a hard cap well under the nextest 300 s backstop.

## Acceptance criteria
- [ ] 24 parallel copies of the rate binary under load: no failure in the harness waits, 3 rounds.
- [ ] fmt, clippy, nextest, deny pass.

## Signals & storage
None.
