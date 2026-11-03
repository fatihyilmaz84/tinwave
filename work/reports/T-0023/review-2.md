REVIEW T-0023 (round 2)
verdict: APPROVE
checks (run by reviewer): fmt pass; clippy (`--workspace --all-targets -D warnings`) pass; tests: `cargo nextest run --workspace` 506 passed / 0 failed / 6 skipped. Stress: `cargo nextest run -p tinwave-playback --stress-count 25 -j 24` over queue_edits, controller_edits, controller_random, pairing, rt_alloc, controller_rt_alloc, controller_thread, controller_signals: 25/25 iterations (81 tests each, 2025 passes). deny ok. `git merge-tree --write-tree main t/T-0023-live-queue-edits` clean (b51a6d0). Worktree at ffe9748, sources clean.

acceptance criteria:
- [x] Round-1 blocker (Remove naming the head): controller/mod.rs:334-338 returns `InvalidArgument` before `plan_edit`, `poll`, `replace_after` and `expect_edit` — nothing sent or recorded. controller_edits.rs:678-683 adds `Remove{[1]}` and `Remove{[3,1]}` and asserts `edits_pending == 0` and no queue signal (715-716). Reviewer mutation (`false && …`) fails `an_edit_that_contradicts_its_own_description_is_rejected_before_it_is_sent` at line 685.
- [x] PlayNext and Reorder can't record against the head: their items must be in `tail` (mapper.rs:268-290); a tail containing the head's id is refused by `Engine::replace_after` (engine.rs:275-279) before it is sent; `expect_edit` runs only after Ok. Covered by `an_edit_with_nothing_playing_or_with_duplicate_ids_is_an_error_not_a_change`. An item before the head is refused by the decoder's uniqueness check (emits nothing).
- [x] P-0198 `last_opened`: set when the mark is published (decoder.rs:445); cleared in `reset_stream` (389), which every restart path goes through; used only when `reader` and `awaiting` are both `None` (336-340); with a pending Load/Seek `awaiting` is Some and the base is `cur` (the target). FIFO race reasoning unchanged.
- [x] `st.cur = base + 1` can't skip or replay: runs only when `reader` and `awaiting` are `None`; then `last_opened < cur` always holds (cur only moves forward until a Load/Seek sets `awaiting` or a restart clears `last_opened`), so it only moves `cur` back over truncated, never-opened entries; in the normal exhausted case `base + 1 == cur`. 0-frame hand-off kept (revive path unchanged). Scratch tests (own CARGO_TARGET_DIR, deleted): [A, missing] + Append C + edit(A → [B]) applied, bit-exact, started [1,4] (when the decoder opened C first, refused with a visible `StaleQueue{current: 3}`, as designed); edit after failed successors to [missing, B] applied, bit-exact, started [1,4]; [A, missing] + edit(A → []) applied, one QueueEnded, started [1].
- [x] New test meaningful: removing `st.cur = base + 1` fails it and two scratch tests ("decoder did not deliver"); reverting the base to `st.cur.checked_sub(1)` fails it and all three scratch tests (`StaleQueue`); unmutated it passed every stress iteration.
- [x] `LoadRequest.leaving` doc (types.rs:66-67).

blocking findings:
- none

design check: n/a

non-blocking findings:
- decoder.rs:94-97 — `last_opened` doc says "`None` until a stream has started"; it is also `None` after a restart in which nothing opened (refused as stale while nothing is audible). Behaviour correct; comment incomplete.
- mapper.rs:293-310 — Remove of an already-played item is still recorded (P-0162, parked).

proposals:
- none

notes for lead: merge clean; re-run merge-tree if T-0026 (adds `mod stretch;` to lib.rs) merges first — this branch doesn't touch lib.rs in round 2. Scratch copy deleted; nothing left running. Not the reviewer's: `cargo nextest run --workspace` PID 25504 (another worktree's agent), an old `python -I …t.py cmd … cmake-build.cmd configure` PID 60496 (2026-11-02).

---
Lead (2026-11-03): merged 5d4742b; the `last_opened` doc comment completed by the lead on main.
