REVIEW T-0005 (round 1)
verdict: APPROVE
checks (run by reviewer): fmt pass (exit 0), clippy `--workspace --all-targets -D warnings` pass (exit 0), tests pass (`cargo test --workspace`: 39+90+9+8+7+6 passed, 0 failed), deny pass ("advisories ok, bans ok, licenses ok, sources ok"). Emit bench gate exit 0: p99 200 ns draining writer, 200 ns evicting, 400 ns with 3 flooders; max 29.5 / 26.4 / 173.3 µs; 0 reserved overflows.

Checks ran in the worktree D:\dev\Tinwave-worktrees\T-0005-signal-bus at b921243. The diff touches only crates/tinwave-signals/**, root Cargo.toml `[workspace.dependencies]` (crossbeam-channel; criterion with default-features=false and cargo_bench_support only) and Cargo.lock. Runtime deps (`cargo tree -e normal`): crossbeam-channel, serde, thiserror, tinwave-core. criterion is a dev-dependency only. No TODOs, no logging, Debug impls print no event contents.

acceptance criteria:
- [x] Producer emit < 50 µs p99 (criterion bench). Verified by `cargo bench -p tinwave-signals --bench emit`. The gate in benches/emit.rs:234-301 times 200 000 calls one by one and calls `exit(1)` if p99 >= 50 µs. Measured p99 is 200–400 ns, about 125x under budget. Windows `Instant` has 100 ns resolution, but the margin is large enough that this does not matter. A preemption costs 26–173 µs and lands in the max, not the p99: it would need more than 1 % of calls to be descheduled, while a call lasts ~200 ns against a ~15 ms quantum. The gate is meaningful and should not be flaky.
- [x] Overflow test: all T0 events survive and the drop counter is correct. Verified by `bus_acceptance::overflow_flood_of_low_priority_events_keeps_every_playback_event_and_counts_drops`: 1 500 `Paused` all delivered in seq order, `dropped(ViewOpened) == 100 000 − 128` exactly. Also covered by proptest `every_emitted_event_is_delivered_or_counted_as_dropped` and the 4-thread test `bus::tests::concurrent_producers_lose_no_reserved_event_and_account_for_every_low_one`. Its outcomes are the same under any interleaving.
- [x] Disabled category ⇒ zero events constructed (counting constructor). Verified by `bus_acceptance::a_disabled_category_constructs_zero_events` (every category × every event kind; the constructor is `unreachable!`), `a_disabled_tier_constructs_zero_events_and_enabled_kinds_still_flow`, `coalesce::tests::a_disabled_category_never_builds_the_tick` (toggle checked at coalesce.rs:369 before `build()`), `counters_respect_the_toggles_too` (check at counters.rs:184, before the clock read and the lock). At flush, `emit_payload` re-checks the toggle (bus.rs:333), so turning a category off mid-gesture emits nothing (`a_category_turned_off_mid_gesture_drops_the_pending_event_unbuilt`).
- [x] Coalescer: 40 slider ticks in 500 ms ⇒ one event with first `from` and last `to`; seeks within 2 s merge. Verified by `bus_acceptance::forty_slider_ticks_in_500_ms_make_one_event_first_from_last_to` and `coalesce::tests::forty_slider_ticks_in_500_ms_become_one_event_with_first_from_and_last_to`. Boundaries: 599 ms holds, 600 ms flushes; system volume 399 ms holds, 400 ms flushes; seeks 1 999 ms merge, 2 000 ms split. Windows 600 / 600 / 400 / 2000 ms, pinned by `the_window_table_has_the_per_kind_values`.
- [x] UI counters: 1 000 impressions on one entity ⇒ one counter row with count 1 000. Verified by `bus_acceptance::one_thousand_impressions_on_one_entity_are_one_counter_row_with_count_1000` (day 20261102) and `counters::tests::a_thousand_impressions_on_one_entity_are_one_row_with_count_1000`.

blocking findings:
- none

Answers to the lead's specific questions:
- Can a producer block? No. Producers only use `try_send`/`try_recv` on crossbeam array channels. Eviction races between producers are benign (possible over-eviction, every evicted event counted, bus.rs:384-386; after 4 failed attempts the newest event is dropped and counted, bus.rs:395). The drain's look-ahead `head` cannot be evicted. Reserved events are never touched by `low_evict`; a full reserved queue rejects the new event and counts it in `reserved_overflows`.
- Can a reserved event be lost or reordered? Lost only on reserved-queue overflow (counted and surfaced). From a single producer thread, reserved events arrive in order.
- Is `seq` atomic and gap-free? Atomic (`fetch_add`) and unique; not gap-free (drops, `KindMismatch`, crash in flight). Acceptable under database.md, but not documented (non-blocking 2).
- Does `pump` merge by seq correctly when one queue lags? Yes within a batch; across a batch boundary two producers can commit out of seq order (documented at bus.rs:33-38).
- Coalescer field rules match the handoff and ADR 0004 §4; no timer threads; `Drop` and `flush_all` emit what is pending.
- Suppressing no-op gestures and net seeks ≤ 2 s is correct per the catalog (">" 2 s); suppressions are counted.
- UiCounters local-day math: Hinnant's algorithm with `div_euclid`/`rem_euclid`; tests cover pre-epoch, leap days 2024/2000, Dec 31 → Jan 1 with +60, Nov 2 → Nov 1 with −60, i64 clamping; T0 constant day 20 759 = 2026-11-02 hand-checked. Delta semantics and key cap correct.
- Sink retry: the bus cannot double-count; a failed batch is re-offered as-is. Double-counting is only possible if T-0006's sink commits counters and then returns `Err` (breaks the one-transaction contract). Not a defect in this branch; contract text must be unambiguous (non-blocking 1) and T-0006 must test it.

design check: n/a (no UI)

non-blocking findings:
1. bus.rs:56 and :67 — retry contract says "should … one transaction" and "On `Err` nothing *may* have been committed"; reword to MUST / all-or-nothing, and say counter rows are not idempotent.
2. bus.rs:33-38 — document that `seq` has gaps (low evictions, reserved overflow, contention drops, `KindMismatch` at :313-318, crash in flight).
3. bus.rs:527-529 — `wait()` returns `true` immediately while a failed batch is retained; a writer loop spins at 100 % CPU while the database keeps failing. Document backoff or honour the timeout.
4. coalesce.rs:208-217 — a merged seek's net distance includes playback between ticks (two 1 s forward nudges 1.5 s apart ⇒ net 3.5 s ⇒ SeekForward emitted; two 1 s back nudges 1.5 s apart ⇒ net −0.5 s ⇒ suppressed). Use the sum of per-tick jumps for threshold and direction; keep first `from` / last `to` as the payload.
5. counters.rs:216-229 — `drain_into` holds the counters mutex while building up to 16 384 rows; make the hold O(1) by swapping in a spare map.
6. benches/emit.rs:285 — ~14 literal spaces mid-sentence in the printed message. Cosmetic.

proposals:
- P: Key `UiCounters` by entity UUID and let the writer resolve dictionary ids | size: S
- P: database.md `ui_counter`: add an entity-kind discriminator or separate dictionaries (recording id 5 and album id 5 collide under PK (day, kind, entity); only dict_recording exists) | size: S
- P: Align rust.md Concurrency wording with the Priority rule ("never drop T0 playback events" vs low-priority T0 kinds) | size: S
- P: Wire the emit p99 gate into CI | size: S

notes for lead:
- T-0006 must: (a) write events and counters in one transaction and return `Err` only when nothing was committed; (b) insert events with `INSERT OR IGNORE` on (device, seq); (c) back off after a sink `Err`; (d) test that a failure mid-batch rolls back the counters and the retry is counted exactly once.
- A future sync protocol must not use "max seq seen" as a cursor (out-of-order commits across batches, gaps).
- `GestureCoalescer` is `&mut self`: in-app volume (UI thread) and system volume (OS provider thread) should each own one with their own bus clone.
- Merge order: if T-0016 or T-0009 merge first, rebase and regenerate Cargo.lock before merging, then re-run deny.

---
Lead (2026-11-02): merged as ee20850. Non-blocking 1-6 and proposals 1-2 -> T-0022 (lands before T-0006);
proposal 2 schema part and proposal 3 fixed by the lead in database.md / rust.md; proposal 4 parked (no CI remote).
