---
id: T-0059
title: Play-end signal for stops and quits, so they get play records
type: feature
epic: E-01
status: backlog
priority: P1
size: M
depends_on: [T-0029]
scope:
  - crates/tinwave-playback/src/controller/
  - crates/tinwave-playback/tests/controller_signals.rs
  - crates/tinwave-signals/src/        # the event kind
  - crates/tinwave-db/src/play.rs      # Stopped/Quit rows
  - crates/tinwave-db/tests/play_records.rs
  - docs/guides/signal-catalog.md
  - apps/tinwave-desktop/              # Signal Center list entry only
owner:
branch:
review_round: 0
created_by: from P-0233 (T-0029 build-1)
---

## Context
T-0029 writes play rows for Completed and Skipped. The controller deliberately emits no preference signal for a stop
below 97 % or at quit ("stopping is not a preference"), so those plays have no end in the signal stream: the writer
cannot write `EndReason::Stopped` (4) / `Quit` (5) rows, and a stop is indistinguishable from a decode failure after
the track was heard.

## Scope
- A neutral play-end event (e.g. `PlayEnded{reason: Stopped|Quit, listened}`) that is **not** a preference signal: it
  closes the play, the taste engine must not score it as a skip.
- The writer pairs it like Completed/Skipped and writes exactly one row with end_reason Stopped / Quit.
- Failed still writes nothing (T-0029 lead decision).
- Hard rule for a new signal: signal catalog entry, Signal Center UI list, privacy tier, storage estimate.

## Acceptance criteria
- [ ] Stop below 97 % and quit each write exactly one play row with the right end_reason and listened time — test.
- [ ] A stop never produces Skipped/Completed and is not counted as a preference — test.
- [ ] A failed track still writes no row — test.
- [ ] Catalog, Signal Center list and privacy tier updated; growth test still passes.

## Added from T-0065 review-1 (P-0282)
- A stale PlayStarted with no row (stopped/failed play) can pair with an orphan outcome of the same recording while
  Queue & library is off (play.rs ~284). With the play-end signal in place, also ignore starts older than the hot window.

## Signals & storage
One new low-frequency event per stopped/quit play (at most one per play).
