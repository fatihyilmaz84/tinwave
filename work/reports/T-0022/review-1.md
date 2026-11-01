REVIEW T-0022 (round 1)
verdict: APPROVE
checks (run by reviewer): fmt pass (exit 0), clippy `--workspace --all-targets -D warnings` pass (exit 0), tests pass (`cargo test --workspace`: 39+101+10+8+7+6 = 171 passed, 0 failed), deny pass ("advisories ok, bans ok, licenses ok, sources ok"). Emit bench gate exit 0: p99 200 ns draining writer, 200 ns evicting, 400 ns with 3 flooders; 0 reserved overflows. Criterion `ui_counter_impression` 116 ns (was ~95 ns: 16-byte key and an extra field).

Checks ran in D:\dev\Tinwave-worktrees\T-0022-signal-bus-hardening at 9b8750d. The handoff matches the diff.

Scope: only bus.rs, coalesce.rs, counters.rs, payload.rs (one doc comment, 280-281), tests/bus_acceptance.rs, benches/emit.rs. lib.rs untouched (`EntityKind` already exported). enums.rs, frozen.rs, codec, event schema and Cargo files untouched; encoding and frozen codes unchanged.

acceptance criteria:
- [x] Items 1-8 done; new tests for 3, 4 and 7; existing tests still pass:
  1. bus.rs `SignalSink` "Contract: all or nothing" (MUST one transaction, `Err` = nothing committed, counters not idempotent; `INSERT OR IGNORE` on (device, seq)).
  2. bus.rs "`seq` has gaps and commits slightly out of order": five gap sources, "a gap is not a loss", "highest seq seen is not a cursor", `first_seq` the only use of max stored seq.
  3. bus.rs:576-580; tests `wait_backs_off_for_the_timeout_while_a_failed_batch_is_retained`, `wait_backs_off_when_only_counter_rows_are_retained`, `wait_is_quick_again_once_the_retained_batch_is_written` (lower bounds on `thread::sleep`, can't flake).
  4. coalesce.rs:117-124, 211-231, 402-413; the four nudge tests.
  5. counters.rs:259-276; `counting_continues_across_drains_and_reuses_the_swapped_tables`, `no_interaction_is_lost_or_doubled_when_draining_while_counting`.
  6. emit.rs:288 fixed.
  7. counters.rs:66-112; `the_same_uuid_under_another_entity_kind_is_another_row`, `a_track_counter_carries_the_recording_uuid_bytes`, `bus_acceptance::counters_are_keyed_by_entity_kind_and_uuid_so_equal_uuids_of_two_kinds_stay_apart`.
  8. payload.rs:280 `dict_entity`.
- [x] Emit p99 gate still passes.
- [x] fmt, clippy, tests, deny pass.

Answers to the lead's checks:
- Item 3 normal path unchanged (queues, then `Select::ready_timeout`; wakes on arrival). With a retained batch, sleeping the full timeout is correct: `pump` re-offers the retained batch first, so queued work can't be written before it succeeds. Cost: first retry after recovery up to one timeout late (≤ one 2 s interval), well within the 2 048-slot reserved queue.
- Item 4: forward example sums to 2.0 s ⇒ suppressed (net 3.5 s); back example −2.0 s ⇒ suppressed; hidden-by-playback −3 s ⇒ SeekBack although net −1.5 s. For a single seek sum == net; for merged seeks ">2 s" now means the sum of jumps. Catalog wording right.
- Item 5: lock order always spare → rows, only in `drain_into`; producers and `len()` take only `rows`; no deadlock. Swap moves all entries at once; nothing lost or doubled; key cap unchanged.
- Item 7 matches database.md (PK (day, kind, entity_kind, entity); Track → dict_recording, else dict_entity; EntityKind frozen codes Track 0, Album 1, Artist 2, Playlist 3; rows sorted by (day, kind, entity_kind.code(), uuid); sink creates dictionary entries in the same transaction).

blocking findings:
- none

design check: n/a (no UI)

non-blocking findings:
1. coalesce.rs:223-231 — direction from summed jumps with payload first `from` / last `to` means a SeekBack can carry `to_ds > from_ds` (e.g. at 1.5× rate two 1.1 s back-nudges 1.9 s apart: sum −2.2 s ⇒ SeekBack, payload 100.0 → 100.65; at 1× three 0.8 s back-nudges 1.9 s apart ⇒ 100.0 → 101.4). Forward seeks can't be affected. Replay-heatmap consumers reading SeekBack as [to, from] would see an inverted interval. Options: suppress and count when sign(to − from) ≠ sign(sum), or document that a merged SeekBack may have to ≥ from (consumers use min/max). Decide before the taste engine consumes seeks; add a test.
2. bus.rs:579 — `wait` uses `thread::sleep`, so while the sink keeps failing the writer can't see producer disconnect until the timeout ends (app quit delayed up to one flush interval, only while the DB fails). Acceptable; one doc line, or `Select` with the timeout and sleep only the remainder.
3. coalesce.rs:862 — rustfmt pushed a trailing comment to column 63; move it above the statement. Cosmetic.

proposals:
- P: Rename `SEEK_MIN_NET_DS` → `SEEK_MIN_TOTAL_DS` (endorsed) | size: S
- P: T-0006 sink-contract tests (endorsed): mid-batch failure rolls back counters and dictionary inserts, retry counts once, `wait` gets the flush interval | size: S

notes for lead:
- Decide non-blocking 1 in the catalog edit.
- T-0006 must pass the flush interval (seconds) as the `wait` timeout.
- The branch touches only crate files; no Cargo.lock conflict with T-0016 from this side.

---
Lead (2026-11-02): merged. Non-blocking 1 decided: suppress-and-count when the net movement disagrees with the summed
jumps, so SeekBack always has to < from and SeekForward to > from (catalog note added; code + test in T-0015).
Non-blocking 2 (doc line) and 3 (comment) also in T-0015.
