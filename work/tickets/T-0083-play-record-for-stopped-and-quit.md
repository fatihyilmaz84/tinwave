---
id: T-0083
title: Play records for stopped and quit plays
type: feature
epic: E-01
status: backlog
priority: P2
size: M
depends_on: [T-0070]
scope:
  - crates/tinwave-db/src/play.rs
  - crates/tinwave-db/src/writer.rs
  - crates/tinwave-playback/src/controller/
  - crates/tinwave-db/tests/   # a new test file
  - docs/rules/storage-budget.md, docs/guides/signal-catalog.md   # end_reason rows only
owner:
branch:
review_round: 0
created_by: from P-0327 (T-0070 build-1)
---

## Context
A play that the listener stops or that ends by quitting the app writes no play row today (play.rs: "Stopped and quit
plays cannot be derived yet"), so `EndReason::Stopped`/`Quit` are never stored and their RateSession is dropped
(`WriterStats::derived_unmatched`).

## Scope
- Derive and write the play row for a stop and for a clean quit (flush on shutdown); `end_reason` Stopped/Quit.
- Their RateSession lands in the row.

## Acceptance criteria
- [ ] A stopped play and a play ended by a clean quit each have one play row with the right end_reason and rate.
- [ ] Storage growth test passes; budget doc updated if the estimate changes.
- [ ] fmt, clippy, nextest, deny, `cargo build --workspace` pass.

## Signals & storage
No new signal kind; more play rows (one per stopped/quit play).
