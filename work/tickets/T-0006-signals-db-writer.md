---
id: T-0006
title: signals.db migrations and batched SignalWriter
type: feature
epic: E-00
status: in_progress
priority: P0
size: M
depends_on: [T-0005, T-0022]
scope:
  - crates/tinwave-db/
  - Cargo.toml            # [workspace.dependencies] only — members glob "crates/*" already covers the crate
  - Cargo.lock
owner: tw-builder (round 2)
branch: t/T-0006-signals-db-writer
review_round: 2
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
- From the T-0005 review: (a) events and counters in ONE transaction, return `Err` only when nothing was
  committed; (b) insert events with `INSERT OR IGNORE` on (device, seq); (c) back off after a sink `Err`;
  (d) test that a failure mid-batch rolls back the counters and the retry counts exactly once.
- Counter rows arrive keyed by (EntityKind, UUID) after T-0022: resolve via `dict_recording` (Track) or
  `dict_entity` (others) with the bounded LRU; migration 0001 includes `dict_entity` and
  `ui_counter.entity_kind` in the primary key (database.md).
- Writer loop passes its flush interval as the `SignalDrain::wait` timeout; `wait` sleeps the full timeout while a
  failed batch is retained, so that interval is also the retry interval (P-0040).
- Coalescers are `&mut self`: in-app volume (UI thread) and system volume (provider thread) each own one with
  their own bus clone.

## Acceptance criteria
- [ ] Migrations apply on an empty DB and are idempotent — integration test.
- [ ] 256-event batch write < 5 ms — bench.
- [ ] 100 000 realistic events ⇒ average on-disk size incl. index ≤ 60 bytes/event — test measures page count.
- [ ] Writer adds < 2 MB RSS — bench note.
- [ ] Duplicate `(device, seq)` insert is ignored (sync dedupe) — test.
- [ ] Shutdown flush loses nothing already accepted by the bus — test.

## Signals & storage
Core of the storage budget. Report measured bytes/event in the handoff.
