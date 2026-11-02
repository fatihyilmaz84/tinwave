---
id: T-0034
title: PlayStarted only when audio is heard; engine publishes the initial StreamInfo
type: feature
epic: E-01
status: backlog
priority: P1
size: M
depends_on: [T-0024]
scope:
  - crates/tinwave-playback/
owner:
branch:
review_round: 0
created_by: from P-0096, P-0097
---

## Context
T-0024: a track that fails to open still produces PlayStarted with no outcome (catalog §4 "Any play start" means a
play the listener heard). The shell must pass the initial output mode by hand because only a device switch emits
`OutputReopened`.

## Scope
- PlayStarted is emitted when the first audio of the track is heard (first PositionMark past the start, or an
  equivalent engine signal), not at TrackStarted; a track that fails before any audio leaves no PlayStarted.
- The engine publishes the initial `StreamInfo` once the output opens (e.g. an `OutputOpened{info}` event or the
  first `OutputReopened`), so `ExclusiveModeToggled` and diagnostics need no hand-passed initial mode.

## Acceptance criteria
- [ ] Failed open ⇒ no PlayStarted; normal play ⇒ exactly one PlayStarted, emitted when audio is heard — tests.
- [ ] Initial StreamInfo is observable without a device switch — test.
- [ ] Pairing, rt_alloc, controller suites, concurrent stress green; fmt, clippy, tests, deny.

## Signals & storage
No new kinds; PlayStarted timing changes.
