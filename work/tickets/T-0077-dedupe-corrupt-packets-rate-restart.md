---
id: T-0077
title: Report each corrupt packet once across a rate restart
type: chore
epic: E-01
status: backlog
priority: P3
size: S
depends_on: [T-0063]
scope:
  - crates/tinwave-playback/src/decoder.rs
  - crates/tinwave-playback/src/reader.rs
  - crates/tinwave-playback/tests/rate.rs
owner:
branch:
review_round: 0
created_by: from P-0305 (T-0063 build-1)
---

## Context
A rate restart re-reads the item from the audible frame, so packets reported before the restart can be counted again.

## Scope
- Track a "reported up to source frame N" per item (or per-packet source frames) so `drop_reader` sends only new ones.

## Acceptance criteria
- [ ] A track with a corrupt packet before the restart point reports it once across one or more restarts (test).
- [ ] fmt, clippy, nextest, deny pass.

## Signals & storage
None.
