---
id: T-0022
title: Signal bus hardening after review — sink contract, retry backoff, seek threshold, UUID-keyed counters
type: chore
epic: E-00
status: in_progress
priority: P0
size: S
depends_on: []
scope:
  - crates/tinwave-signals/src/bus.rs
  - crates/tinwave-signals/src/coalesce.rs
  - crates/tinwave-signals/src/counters.rs
  - crates/tinwave-signals/src/payload.rs   # playlist_id doc comment only
  - crates/tinwave-signals/src/lib.rs        # re-exports only
  - crates/tinwave-signals/tests/bus_acceptance.rs
  - crates/tinwave-signals/benches/emit.rs
owner: tw-builder (round 1)
branch: t/T-0022-signal-bus-hardening
review_round: 0
created_by: from T-0005 review-1 non-blocking findings, P-0034, P-0035
---

## Context
T-0005 review (work/reports/T-0005/review-1.md, non-blocking findings 1-6 and proposals). T-0006 (signals.db writer)
implements `SignalSink` and builds on this, so it lands first. database.md now has `dict_entity` and
`ui_counter.entity_kind` in the primary key (P-0035).

## Scope
1. `SignalSink` contract text: the write MUST be one transaction; on `Err` nothing has been committed (all-or-nothing);
   counter rows are not idempotent, so a partial commit followed by `Err` double-counts.
2. Document that `seq` has gaps (low evictions, reserved overflow, contention drops, `KindMismatch`, crash in flight)
   and can commit slightly out of order across batches — neither the writer nor a sync cursor may treat a gap as loss
   or use "max seq seen" as a cursor.
3. `SignalDrain::wait` must not spin while a failed batch is retained: honour the timeout (or a documented backoff)
   when only a retained batch is pending. Test: a sink that always fails does not make `wait` return immediately.
4. Seek coalescing: threshold and direction use the sum of the user's jumps (per tick `to − from`), not the net
   position change that includes playback between ticks; payload keeps first `from` and last `to`. Tests for both
   reviewer examples (two 1 s forward nudges 1.5 s apart ⇒ suppressed; two 1 s back nudges ⇒ suppressed).
5. `UiCounters::drain_into`: O(1) hold of the mutex (swap in a spare map), then build rows outside the lock.
6. Bench message cosmetic fix (emit.rs:285).
7. `CounterEntity` becomes `(EntityKind, entity UUID)` (P-0034): UI producers key counters by UUID; the writer resolves
   dictionary ids (dict_recording for Track, dict_entity otherwise). `CounterRow` carries `entity_kind` and the UUID.
   Key-cap behaviour unchanged.
8. `payload.rs` `playlist_id` doc: dictionary id in `dict_entity` (database.md).

## Out of scope
tinwave-db (T-0006), enums.rs / frozen.rs / Cargo files (T-0015), the schema or encoding of events.

## Acceptance criteria
- [ ] Items 1-8 done; new tests for 3, 4 and 7; existing bus/coalescer/counter tests still pass.
- [ ] Emit p99 gate still passes (`cargo bench -p tinwave-signals --bench emit`).
- [ ] fmt, clippy `-D warnings`, tests, `cargo deny check` pass.

## Signals & storage
No new kinds. Counter rows are keyed by (day, kind, entity_kind, entity) per database.md.
