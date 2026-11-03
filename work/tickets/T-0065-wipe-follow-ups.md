---
id: T-0065
title: Wipe follow-ups — cache invalidation only on wipe, rollup plays kept, stale-start guard, WAL checkpoint
type: chore
epic: E-01
status: done
priority: P1
size: S
depends_on: [T-0061]
scope:
  - crates/tinwave-db/
  - docs/rules/privacy-and-signals.md   # wipe paragraph only
owner: tw-builder
branch: t/T-0065-wipe-follow-ups
review_round: 1
merge: 4950c97
created_by: from P-0259..P-0262 (T-0061 review-1)
---

## Context
work/reports/T-0061/review-1.md non-blocking findings.

## Scope
- **Cache invalidation (P-0259):** `SignalStore::write_batch` clears its dictionary/session caches on any change of
  `PRAGMA data_version`, i.e. on every other-connection commit (~+30 % writer CPU in the 90-day harness, which writes
  snapshots on its own connection). Clear only after a wipe: e.g. the wiper bumps a wipe epoch the writer reads in the
  same per-batch query, or on a version change probe one cached id. Use `prepare_cached` for the per-batch read.
- **Rollup plays (P-0260, lead decision):** the Seek/skip/replay wipe keeps `rollup_monthly.plays` (only Queue & library
  clears it), matching "keeps what was played and when" in the warm tier. Update the rule table and tests.
- **Stale start guard (P-0261):** pairing ignores a PlayStarted that already has a play row, or one older than the hot
  window, so a post-wipe outcome can't pair with a stale start (crates/tinwave-db/src/play.rs ~203).
- **Checkpoint (P-0262):** after a wipe the wiper runs a best-effort `wal_checkpoint(TRUNCATE)` with a zero busy
  timeout; the doc sentence about `secure_delete` then drops its "once checkpointed" caveat if the test proves it.

## Acceptance criteria
- [ ] Writer CPU in the 90-day growth run back to the pre-T-0061 level (±10 %), and a wipe still invalidates the
      caches (the T-0061 writer tests still pass; mutation shows they fail without invalidation) — measured + tests.
- [ ] Seek wipe keeps `plays`, Queue & library wipe clears it — test.
- [ ] Stale-start scenario (seek wipe, Queue & library off, same recording's outcome) writes no duplicate row — test.
- [ ] The wiped marker is absent from the main DB file right after the wipe returns (no manual checkpoint in the test)
      when no reader blocks the checkpoint — test.
- [ ] fmt, clippy, nextest, deny pass.

## Signals & storage
None.
