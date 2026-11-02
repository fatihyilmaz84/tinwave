HANDOFF T-0022 (round 1)
status: done
branch: t/T-0022-signal-bus-hardening   commit: 9b8750d
summary: All eight items are done inside the ticket's file list. The sink contract and the seq-gap rules are documented in bus.rs. `wait` now backs off while a failed batch is retained. Seek threshold and direction come from the summed per-tick jumps. `drain_into` swaps maps under the lock. `CounterEntity` is now (`EntityKind`, 16-byte UUID) and `CounterRow` carries `entity_kind` and the UUID. The emit p99 gate still passes.

acceptance criteria:
- [x] Items 1-8 done; new tests for 3, 4 and 7; existing tests still pass — `cargo test --workspace` 39+101+10+8+7+6 passed, 0 failed.
- [x] Emit p99 gate still passes — reserved/draining p99 200 ns (p99.9 300 ns, max 176.6 µs); low/evicting p99 200 ns (max 22.1 µs); reserved + 3 flooders p99 400 ns (max 45.4 µs), 0 reserved overflows.
- [x] fmt, clippy `-D warnings`, tests, `cargo deny check`; `RUSTDOCFLAGS="-D warnings" cargo doc -p tinwave-signals --no-deps` clean.

Per item:
1. `SignalSink` "Contract: all or nothing": one call MUST be one transaction; `Err` = nothing committed; counter rows not idempotent. `write` docs: `INSERT OR IGNORE` on (device, seq); sink resolves dictionary ids (`dict_recording` for Track, `dict_entity` otherwise).
2. Module section "`seq` has gaps and commits slightly out of order": eviction, contention drop, reserved overflow, `KindMismatch`, crash in flight; a gap is not a loss; "max seq seen" is not a cursor; only `BusConfig::first_seq` may use the highest stored seq.
3. `SignalDrain::wait` sleeps the full `timeout` and returns `true` whenever a batch is retained (also when queues hold more work). Tests: `wait_backs_off_for_the_timeout_while_a_failed_batch_is_retained` (DeadSink, 4 rounds ≥ 4×40 ms wall-clock lower bound), `wait_backs_off_when_only_counter_rows_are_retained`, `wait_is_quick_again_once_the_retained_batch_is_written`.
4. `Gesture::jump_ds()` (signed `to − from` for seeks) summed in `Pending`; `finish(span_ms, jump_ds)` uses the sum for threshold (`<= SEEK_MIN_NET_DS`) and direction; payload first `from` / last `to`. Tests: `two_one_second_forward_nudges_1500_ms_apart_are_not_a_seek`, `two_one_second_back_nudges_1500_ms_apart_are_not_a_seek`, `nudges_that_add_up_to_more_than_two_seconds_are_one_seek_with_first_from_and_last_to`, `back_nudges_are_a_seek_when_the_jumps_add_up_even_if_playback_hides_them_in_the_net`; renamed `a_merged_seek_takes_its_kind_from_the_direction_of_the_summed_jumps`, `a_seek_of_two_seconds_or_less_is_not_a_seek`.
5. `UiCounters` spare map: `drain_into` takes the spare lock (producers never take it), swaps with the live map under the hot mutex, builds rows outside the lock. Tests: `counting_continues_across_drains_and_reuses_the_swapped_tables`, `no_interaction_is_lost_or_doubled_when_draining_while_counting` (3 threads).
6. Bench message spaces removed.
7. `CounterEntity { kind: EntityKind, uuid: [u8; 16] }` (`new`, `track(RecordingId)`, Ord, Hash); flat `CounterRow { day, kind, entity_kind, entity: [u8; 16], count, total_ms, max_scrolled }`; rows sorted by (day, kind, entity_kind code, uuid); key cap unchanged; `EntityKind` reused from enums.rs unchanged. Tests: `the_same_uuid_under_another_entity_kind_is_another_row`, `a_track_counter_carries_the_recording_uuid_bytes`, `bus_acceptance::counters_are_keyed_by_entity_kind_and_uuid_so_equal_uuids_of_two_kinds_stay_apart`.
8. payload.rs `playlist_id` doc → `dict_entity` id.

checks: fmt pass, clippy pass, tests 171 passed / 0 failed, deny pass
files changed: crates/tinwave-signals/src/bus.rs, src/coalesce.rs, src/counters.rs, src/payload.rs (doc only), tests/bus_acceptance.rs, benches/emit.rs
signals added/changed: none; counter rows keyed (day, kind, entity_kind, entity) per database.md.
ui: none
docs updated: module docs only
deviations from ticket: none. test_support.rs untouched (compiles as is). `SEEK_MIN_NET_DS` keeps its name/export (now thresholds the summed jumps; doc says so).

proposals:
- P: Rename `SEEK_MIN_NET_DS` to `SEEK_MIN_TOTAL_DS` | size: S | evidence: coalesce.rs:60, lib.rs:36
- P: Signal catalog wording for seek: ">2 s" = sum of the listener's jumps | size: S
- P: T-0006 tests for the sink contract (rollback mid-batch, retry counts once; `wait` timeout is the retry interval) | size: S

questions for lead: none. Note for T-0006: pass the flush interval (seconds) as the `wait` timeout.
