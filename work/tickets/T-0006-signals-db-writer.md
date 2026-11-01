---
id: T-0006
title: signals.db migrations and batched SignalWriter
type: feature
epic: E-00
status: backlog
priority: P0
size: M
depends_on: [T-0005]
scope:
  - crates/tinwave-db/
  - Cargo.toml            # [workspace.dependencies] only — members glob "crates/*" already covers the crate
  - Cargo.lock
owner:
branch:
review_round: 0
created_by: lead
---

## Context
docs/rules/database.md (schema, write path, pragmas), docs/rules/storage-budget.md (≤ 60 bytes/event).

## Scope
- `tinwave-db` crate with a migration runner (numbered SQL files, transactional, version table).
- `signals.db` migration 0001: dictionaries, `session`, `signal_event`, `context_snapshot`, `play`,
  `rollup_monthly`, `ui_counter` exactly as in database.md; pragmas (WAL, incremental auto-vacuum, journal limit).
- `SignalStore` implementation of the trait from `tinwave-signals`; `SignalWriter` thread draining the bus:
  flush every 2 s or 256 events, immediate flush on play-end; dictionary LRU cache; UI counter upserts.
- Graceful shutdown flush.
- Implements T-0005's `SignalSink` (`write(events, counters)`, one transaction per batch; counter rows are deltas:
  ADD `count`/`total_ms`, keep the larger `max_scrolled`). On startup pass `BusConfig::first_seq = max(seq)+1` for
  the device; the session starter calls `bus.set_tz_offset`. `ui_counter.max_scrolled` per database.md (P-0030).
- Typed sink errors (P-0032): keep the database error's source chain instead of a bare string.

## Acceptance criteria
- [ ] Migrations apply on an empty DB and are idempotent — integration test.
- [ ] 256-event batch write < 5 ms — bench.
- [ ] 100 000 realistic events ⇒ average on-disk size incl. index ≤ 60 bytes/event — test measures page count.
- [ ] Writer adds < 2 MB RSS — bench note.
- [ ] Duplicate `(device, seq)` insert is ignored (sync dedupe) — test.
- [ ] Shutdown flush loses nothing already accepted by the bus — test.

## Signals & storage
Core of the storage budget. Report measured bytes/event in the handoff.
