---
id: T-0084
title: Budget and statement-count pins for derived values; small T-0070 tidy-ups
type: chore
epic: E-01
status: backlog
priority: P3
size: S
depends_on: [T-0070, T-0075]
scope:
  - crates/tinwave-db/tests/
  - crates/tinwave-db/src/play.rs          # UPDATE_PLAY_RATE literal formatting only
  - crates/tinwave-signals/src/bus.rs      # SignalSink::write_with_derived default
  - docs/rules/storage-budget.md, docs/guides/signal-catalog.md   # play-record totals only
owner:
branch:
review_round: 0
created_by: from P-0331, P-0332 and the non-blocking findings of T-0070 review-1
---

## Context
T-0070 stores RateSession in `play.rate`, riding the play-end transaction. The guard that it adds no commit is only a
writer unit test; the growth run prints 11 471 transactions but doesn't assert it. The batch statement-count pin
doesn't cover the derived UPDATE. `SignalSink::write_with_derived`'s default silently discards values. The per-play
size totals (~58 B, ~6 KB/day) were not bumped.

## Scope
- Growth test: assert the transaction count equals a run without derived values (or pin it with a stated reason).
- Statement-count test: one statement per derived value, independent of stored rows.
- `write_with_derived`: make it required, or count what the default discards.
- `UPDATE_PLAY_RATE` as a readable multi-line literal like the other SQL constants.
- storage-budget.md and catalog §9 totals updated to the measured values.

## Acceptance criteria
- [ ] A mutation that commits derived values separately fails the growth test.
- [ ] A per-row mutation in the derived UPDATE fails the statement-count test.
- [ ] fmt, clippy, nextest, deny, `cargo build --workspace` pass.

## Signals & storage
None new; docs totals only.
