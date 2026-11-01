HANDOFF T-0005 (round 1)
status: done
branch: t/T-0005-signal-bus   commit: a4ceaec
summary: Added the signal bus to tinwave-signals: SignalBus (producer handle), SignalDrain (single consumer), the SignalSink trait, SignalSettings toggles, UiCounters, and GestureCoalescer. Reserved (T0 playback) events have their own bounded queue and are never evicted; the low-priority queue evicts its oldest event and counts each drop per kind. Producer emit p99 is about 200 ns against the 50 µs budget. No async runtime: producers are plain synchronous code, and the consumer is one writer thread that blocks in `SignalDrain::wait`.

acceptance criteria:
- [x] Producer emit < 50 µs p99, criterion bench. Proof: `cargo bench -p tinwave-signals --bench emit` runs criterion groups, then a gate that times 200 000 calls individually and exits non-zero if p99 is 50 µs or more. Reserved with draining writer p50 100 ns / p99 200 ns; low queue full evicting every call p99 200 ns; reserved with 3 flooder threads p99 300 ns. Criterion means: 104 ns (reserved), 100 ns (evicting), 8.7 ns (disabled), 95 ns (counter), 44 ns (coalescer tick). Single-call max 140–178 µs (OS preemption); p99.9 < 1 µs.
- [x] Overflow test: all T0 events survive and the drop counter is exact — `bus_acceptance::overflow_flood_of_low_priority_events_keeps_every_playback_event_and_counts_drops` (100 000 low events into 128 slots interleaved with 1 500 `Paused`: all `Paused` arrive in order, reserved overflows 0, `ViewOpened` drops = 100 000 − 128 exactly, newest 128 kept). Also `the_newest_low_priority_events_are_the_ones_kept`, proptest `every_emitted_event_is_delivered_or_counted_as_dropped`, and the 4-thread test in `bus::tests`.
- [x] Disabled category ⇒ zero events constructed (counting constructor) — `bus_acceptance::a_disabled_category_constructs_zero_events`; tiers in `a_disabled_tier_constructs_zero_events_and_enabled_kinds_still_flow`; counters in `counters_respect_the_toggles_too`; coalescer in `coalesce::tests::a_disabled_category_never_builds_the_tick`.
- [x] Coalescer: 40 slider ticks in 500 ms ⇒ one event, first `from`, last `to`; seeks within 2 s merge — `bus_acceptance::forty_slider_ticks_in_500_ms_make_one_event_first_from_last_to`; 599 ms silence does not flush, 600 ms does; 1 999 ms gap merges, 2 000 ms splits.
- [x] UI counters: 1 000 impressions ⇒ one row with count 1 000 — `bus_acceptance::one_thousand_impressions_on_one_entity_are_one_counter_row_with_count_1000` (day 20261102 under the test clock).

checks: fmt pass, clippy pass (`--workspace --all-targets -D warnings`), tests 159 passed / 0 failed (workspace; 90 lib + 9 bus_acceptance in this crate), deny pass
files changed: Cargo.toml ([workspace.dependencies]: crossbeam-channel, dev-only criterion without default features), Cargo.lock, crates/tinwave-signals/Cargo.toml, src/lib.rs, src/error.rs (new variants KindMismatch, InvalidConfig, Sink — not a schema change), new src/bus.rs, priority.rs, settings.rs, counters.rs, coalesce.rs, test_support.rs (cfg(test)), tests/bus_acceptance.rs, benches/emit.rs
signals added/changed: none. `SignalDropped` meta diagnostic is the in-memory `SignalBus::dropped(kind)`, `dropped_by_kind()`, `reserved_overflows()`.
ui: none
docs updated: none
deviations from ticket: none. Decisions where the ticket was silent:
- Windows table (coalesce.rs): VolumeChanged/RateChanged 600 ms, SystemVolumeChanged 400 ms, SeekForward/SeekBack 2 000 ms; tested per kind.
- Reserved = all event kinds of catalog §1–4 and 8 (all T0); low = ViewOpened, WindowState, OutputDeviceChanged, EqChanged, ExclusiveModeToggled, CrossfadeChanged. No wildcard arm; list pinned by a test.
- Reserved queue full is a bug, not a policy: new event rejected, `EmitOutcome::Dropped`, `reserved_overflows()` += 1. Defaults 2 048 reserved / 512 low (~0.35 MB).
- Two `crossbeam_channel::bounded` queues; producers evict the oldest low event via a second receiver; `SignalDrain::pump` merges by `seq`.
- `seq` assigned at emit from `BusConfig::first_seq`; T-0006 passes max stored seq + 1. Constructor closure receives `EventStamp { id, ts }`.
- `SignalSink::write(&mut self, events: &[SignalEvent], counters: &[CounterRow]) -> Result<(), SignalsError>`; on Err the drain re-offers the batch; `(device, seq)` is the dedupe key; counter rows are deltas (ADD count/total_ms, keep larger max_scrolled).
- Counter day from `TzOffsetMin` in `BusConfig` / `SignalBus::set_tz_offset`; local yyyymmdd via civil-from-days, no new date dependency.
- `CounterEntity(u32)` = signals.db dictionary id resolved by the caller.
- `UiCounters` key cap (default 16 384): new keys dropped and counted when full.
- Merged gestures: from = first tick; to/input/section/since_* = last; ts/pos_ds/ctx = last; gesture_ms = first-to-last; no-op gestures and net seeks ≤ 2 s emit nothing (counted in `suppressed()`); merged seek direction by net delta.
- Coalescer owns no timer: host calls `flush_due()` at `next_deadline()` (or every 100 ms), `flush_all()` on track change/stop; drop flushes.
- Toggles: seven `SignalCategory` entries (catalog sections 1–6, 8) × T0–T3 tier toggles; defaults T0, T1 on, T2, T3 off; no per-kind toggle.

proposals:
- P: Fix the coalescing window text in storage-budget.md §2 (600 vs ADR 0004's 400 ms) | size: S
- P: Add `max_scrolled` to the `ui_counter` table in database.md | size: S
- P: Persist `SignalSettings` (settings row + load/save) | size: S
- P: A typed sink error instead of `Sink { reason: String }` | size: S
- P: Add the bus bench to the CI regression gate | size: S

questions for lead:
- Should `OutputDeviceChanged` (T1, ambient) stay low priority?
- T-0006 should pass `BusConfig::first_seq` as `max(seq) + 1` for the device, and call `bus.set_tz_offset` on session start. Confirm.

---
Lead answers (2026-11-02), sent before review:
- OutputDeviceChanged → reserved (catalog §1 interprets volume events relative to it); amend requested.
- first_seq / set_tz_offset division confirmed; recorded in T-0006.
- All other decisions accepted. Docs proposals fixed by the lead on main (P-0029, P-0030).

---
HANDOFF T-0005 (round 1, amended)
status: done
branch: t/T-0005-signal-bus   commit: b921243
summary: OutputDeviceChanged is now reserved priority. Only crates/tinwave-signals/src/priority.rs changed. Rule text: "catalog 1-4 and 8, plus kinds whose loss would change how a reserved event is interpreted (OutputDeviceChanged)". Pinned low list: ViewOpened, WindowState, EqChanged, ExclusiveModeToggled, CrossfadeChanged. Tier test every_reserved_kind_is_a_t0_event_except_the_named_interpretation_inputs allows RESERVED_NON_T0 = [(OutputDeviceChanged, T1)] by name.
checks: fmt pass, clippy pass, tests 159 passed / 0 failed, deny pass; emit bench gate pass (p99 200 ns draining, 200 ns evicting, 400 ns with 3 flooders).
files changed: crates/tinwave-signals/src/priority.rs
deviations: none; proposals: none; questions: none
