---
id: T-0029
title: Write play records live when a play ends (warm tier)
type: feature
epic: E-01
status: in_progress
priority: P1
size: M
depends_on: [T-0006, T-0024]
scope:
  - crates/tinwave-db/
  - crates/tinwave-signals/src/   # ContextKey packing type only
  - docs/guides/taste-engine.md   # ContextKey layout section
  - docs/rules/storage-budget.md  # context_key line only
owner: tw-builder
branch: t/T-0029-live-play-records
review_round: 1
created_by: from P-0071
---

## Context
database.md "Write path": play records are written live when a play ends, so the warm tier is always current and the
taste engine never scans raw events. T-0006 creates the `play` table and flushes at play end, but derives no rows.

## Scope
- Derive one `play` row per play from the play's events (started_ts, listened_ds, end_reason, end_frac, flags,
  peak_section, context_key) in the same transaction as the play-end batch.

## Acceptance criteria
- [ ] Each play end (Completed, Skipped*, Stopped, Quit) writes exactly one `play` row in the same transaction — test.
- [ ] A failed track (`TrackLeft{Failed}`) writes **no** play row (lead decision below) — test.
- [ ] `context_key` is packed by a `ContextKey` type with a documented, versioned bit layout; pack/unpack round-trip
  and "unknown" values are tested; a play whose context was never captured stores the all-unknown key (0), not a guess.
- [ ] `peak_section` is NULL (no audio analysis yet; the column is nullable by design, E-03 fills it).
- [ ] Growth test (T-0007) accounts for play rows.

## Lead decisions (2026-11-03)
- **Failed plays write no row.** A decode/IO failure is not a listening choice; counting it as a skip would teach the
  taste engine a false dislike. The `TrackLeft{Failed}` event stays in the hot tier for diagnostics.
- **ContextKey layout** (u32, bit 0 = LSB), every field 0 = unknown:
  - bits 0-2 daypart (the clock provider's dayparts; at most 7 known values), bits 3-4 weekday type (workday/weekend/holiday),
  - bits 5-10 weather (condition 3 bits x temperature band 3 bits from context-engine.md `WeatherBucket`),
  - bits 11-13 device class, bits 14-17 activity (`ActivityGuess`, incl. the mobile values), bits 18-23 place slot (small per-device index,
    never an SSID or MAC), bits 24-27 reserved (0), bits 28-31 layout version (1).
  If a listed enum has more values than its field, stop and report it in the HANDOFF rather than widening silently.
  The context fields come from the events already recorded for the play (the snapshot the bus attached); if none is
  available in the writer, report `partial` with what is missing instead of inventing a source.

## Signals & storage
Warm tier per storage-budget.md.

## Added from P-0111 (T-0024 review)
A play's Completed/Skipped can land in a later session than its PlayStarted (stale play after a 30-min gap). Pair a play start with its outcome per device (by seq order), never per session; test it.

## Added from P-0127 (T-0032 review)
In crates/tinwave-db/src/settings.rs test `the_mask_is_read_under_the_connection_lock`, send `go` before the in-scope asserts (or a drop guard) so a failure cannot hang the test; tidy the trigger SQL literal whitespace in tests/signal_settings.rs:452. Scope extends to those two spots.
