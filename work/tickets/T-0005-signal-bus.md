---
id: T-0005
title: Signal bus with priority drop policy, category toggles, and UI counters
type: feature
epic: E-00
status: done
priority: P0
size: M
depends_on: [T-0004, T-0013]
scope:
  - crates/tinwave-signals/
  - Cargo.toml            # [workspace.dependencies] only
  - Cargo.lock
owner: tw-builder (round 1)
branch: t/T-0005-signal-bus
review_round: 1
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
- Injected `Clock` from `tinwave-core` (T-0013).

## Acceptance criteria
- [ ] Producer emit < 50 µs p99 — criterion bench.
- [ ] Overflow test: flood with low-priority events; all T0 events survive; drop counter correct.
- [ ] Disabled category ⇒ zero events constructed (test with a counting constructor).
- [ ] Coalescer: 40 slider ticks in 500 ms ⇒ one event with first `from` and last `to`; seeks within 2 s merge.
- [ ] UI counters: 1 000 impressions on one entity ⇒ one counter row with count 1 000.

## Signals & storage
Implements counters and coalescing that keep the storage budget.

## Merged
2026-11-02, merge commit ee20850 (branch head b921243). Review: review-1 (APPROVE). Gates green on main (159 tests); emit p99 gate 200-400 ns. Non-blocking findings -> T-0022.
