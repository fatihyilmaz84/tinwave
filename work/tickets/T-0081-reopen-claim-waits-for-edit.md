---
id: T-0081
title: The exclusive reopen claim waits for an unanswered queue edit
type: fix
epic: E-01
status: backlog
priority: P3
size: S
depends_on: [T-0068]
scope:
  - crates/tinwave-playback/src/engine.rs
  - crates/tinwave-playback/src/shared.rs
  - crates/tinwave-playback/src/decoder.rs
  - crates/tinwave-playback/tests/exclusive_reopen.rs
owner:
branch:
review_round: 0
created_by: from P-0322, P-0323 (T-0068 build-1)
---

## Context
T-0068 lets a queue edit withdraw an exclusive boundary park through a compare-exchange on the reopen request. When the
old stream has played out while an edit is queued but not yet taken by the decoder, the engine wins the claim and opens
a stream at the old successor's rate; the decoder then re-parks on `Resume`. Correct, but one wasted open (a gap).

## Scope
- An edit-acknowledged counter (or equivalent) in `Shared`; `service_reopen` does not claim while a `replace_after` is
  sent but unanswered, bounded so a stuck decoder cannot hold the reopen forever.
- A deterministic test of the claim-first interleaving (a test hook or a fake that delays the decoder's edit).

- (P-0324) Cover the engine's lost-claim branch in `service_reopen` (engine.rs ~915): the mutation
  `if !take_reopen(..) && id == u32::MAX` must fail a test.

## Acceptance criteria
- [ ] An edit sent before the drain ends never causes an open at the withdrawn rate (test).
- [ ] A decoder that never answers does not block the reopen beyond the stated bound (test).
- [ ] fmt, clippy, nextest, deny, `cargo build --workspace` pass.

## Signals & storage
None.
