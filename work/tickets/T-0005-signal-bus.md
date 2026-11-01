---
id: T-0005
title: Signal bus with priority drop policy, category toggles, and UI counters
type: feature
epic: E-00
status: backlog
priority: P0
size: M
depends_on: [T-0004]
scope:
  - crates/tinwave-signals/
owner:
branch:
review_round: 0
created_by: lead
---

## Context
docs/rules/rust.md (Concurrency), docs/rules/storage-budget.md §1–2 (counters, coalescing),
docs/rules/privacy-and-signals.md (toggles at the source), docs/rules/testing.md (budgets).

## Scope
- `SignalSink` trait + `SignalBus` on a bounded channel; producers never block.
- Overflow policy: drop oldest low-priority events, never T0 playback events (reserved capacity); count drops.
- `SignalSettings` category/tier toggles checked **before** an event is constructed (`bus.enabled(kind)`).
- `UiCounters`: in-memory aggregation for Counter-class signals keyed by `(day, kind, entity)`, drained by the writer.
- `GestureCoalescer` for volume/rate/system-volume (600 ms) and seek merging (2 s).
- Injected `Clock`.

## Acceptance criteria
- [ ] Producer emit < 50 µs p99 — criterion bench.
- [ ] Overflow test: flood with low-priority events; all T0 events survive; drop counter correct.
- [ ] Disabled category ⇒ zero events constructed (test with a counting constructor).
- [ ] Coalescer: 40 slider ticks in 500 ms ⇒ one event with first `from` and last `to`; seeks within 2 s merge.
- [ ] UI counters: 1 000 impressions on one entity ⇒ one counter row with count 1 000.

## Signals & storage
Implements counters and coalescing that keep the storage budget.
