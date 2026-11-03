REVIEW T-0068 (round 2)
verdict: APPROVE
checks (run by reviewer): fmt pass (exit 0), clippy pass (`--workspace --all-targets -D warnings`, exit 0), tests pass (nextest workspace: "838 tests run: 838 passed (4 slow), 7 skipped", exit 0; stress `nextest run -p tinwave-playback --stress-count 6`: "6/6 stress run iterations: 6 passed", exit 0), deny pass ("advisories ok, bans ok, licenses ok, sources ok", exit 0), `cargo build --workspace` pass (exit 0). rt_alloc (4 tests), output_rt_alloc and controller_rt_alloc all pass. I ran every gate with the Qt env set.

acceptance criteria:
- [x] [a 44.1k, b 96k] parked for b, play-next c 44.1k: no 96 kHz open, a→c gapless, bit-exact, no silence. Verified by `exclusive_reopen::a_play_next_during_a_boundary_park_keeps_a_same_rate_hand_off_gapless_and_opens_no_stream`, which passes. Round 2 changed only a doc comment in decoder.rs, so my round-1 mutation result still applies.
- [x] An edit to another rate re-parks for that rate. Verified by `an_edit_to_a_track_of_another_rate_parks_again_for_that_rate`, which passes.
- [x] Stable under parallel stress; fmt, clippy, nextest and deny pass. Verified by my own runs above, on a machine shared with the T-0070 review and the T-0075 builder.
- [x] Scope item P-0294: the position follows what is heard and never goes back. This was the round-1 blocker and it is fixed. Evidence:
  - `a_same_rate_edit_in_the_drain_does_not_put_the_position_back` and `the_position_follows_the_device_buffer_while_the_old_stream_plays_out_and_never_goes_back` both pass.
  - I re-ran the builder's mutation in a scratch copy (engine.rs:1160 `fetch_max(shown)` changed to `let _ = shown;`). The same-rate test fails with exactly the round-1 numbers: "the position went back: Position { item: QueueItemId(1), frame: 23695 } then Position { item: QueueItemId(1), frame: 23478 }".

blocking findings:
- none

Analysis of the fix, engine.rs:173, :927 and :1151-1161:
- **The floor cannot run ahead of what is heard.**
  - The floor only takes values that `shown` already had. `shown` is clamped to `consumed`, and `consumed` only grows (renderer.rs:233 is its only production writer, a `fetch_add`). So the floor is always at most `consumed`.
  - The drain credit is `idle.min(latency)`. Idle frames only count while the ring is empty, and the count is reset by the next popped frame (renderer.rs:107-114).
  - Same-rate withdrawal: the device FIFO then holds (latency−k) tail frames plus k frames of silence. The real heard position is ≥ max(played, F), so the floor stays at or behind what is heard. It moves forward once c's frames come out.
  - While paused at level 0 the renderer returns before counting idle, so the credit freezes. That is conservative.
- **Track change, seek, stop, device switch, new park.** No reset is needed, because the floor is in consumed-space and is at most the consumed value at the time it was raised.
  - Every later mark (seek, load or switch flush, new track) starts at S ≥ that consumed value.
  - So F < S. `resolve` (engine.rs:1234) gives the same mark and offset for `shown` = F as it would for `played`, both below S.
  - A new park with a different total only credits once `consumed >= total`, and the idle count was reset at that last pop.
  - Device switch during a drain: the restart frame comes from `position()`, so the unheard (latency−k) tail is replayed on the new device. That is correct.
- **Orderings.** `heard_floor` belongs to the engine and is only touched by it. `fetch_max` is an atomic read-modify-write with a single modification order, so Relaxed keeps it monotonic. The view reads `consumed` before the reopen slot and the total. A park that races the view therefore sees an older, smaller `consumed`, and the credit can only come out smaller, never larger.
- **Cost.** `position()` gains one atomic load, plus one `fetch_max` during a drain only. That is on the control thread. renderer.rs is unchanged on the branch, so nothing new runs on the render callback, and the rt_alloc tests pass.

On the two other variants:
- **Other rate:** under the mutant it failed in 20 of 20 runs here with the same 23695 → 23478 message. The re-park happens on the decoder thread after `QueueEdited`, so the test thread usually reads the position first. With the fix the test is deterministic. It is a useful test that is not guaranteed to catch this mutant, and nothing about it is misleading. Acceptable.
- **Remove:** under the mutant it passes, because `ended` snaps `shown` to `consumed` and the position moves forward. Its name is accurate (it checks that the position does not go back), and the builder's handoff says it does not catch the mutant. It is a behaviour check, not a regression guard for this fix. Acceptable.

Merge: main has only work/ commits since fba8f71 (now at 6bbeee3). `git merge-tree --write-tree main HEAD` is clean (exit 0, tree 8d66cc3). The branch changes only the 4 files in scope (shared.rs limited to `take_reopen`). No work/ edits and no TODOs.

design check: n/a (no UI)

non-blocking findings:
- crates/tinwave-playback/src/engine.rs:927: my mutation `heard_floor.fetch_max(total)` → `let _ = total;` survived all 22 exclusive_reopen tests and all 114 lib tests.
  - The line is still needed: `poll()` runs `service_reopen` (engine.rs:714) before `view` (:721). A claim can therefore happen in a poll whose drain no `view` has fully credited yet.
  - Without the line, the position would lag a little behind the end of a until the new stream is heard. It would not go back.
  - A test that claims right after idle crosses latency, with no `position()` read in between, would cover it.
- crates/tinwave-playback/src/engine.rs:915 lost-claim branch: still has no coverage. It is tracked in T-0081, as agreed in round 1.

proposals:
- P: Test that the claim raises the floor to the full total when no view credited the end of the drain | why: kills the surviving mutant at engine.rs:927 | size: S

notes for lead:
- Nothing blocks the merge.
- My scratch copy (made with `git archive`, with its own target dir under the session scratchpad) has been deleted. The worktree is untouched (`git status` clean). Nothing I started is still running.
- Gate logs:
  - C:\Users\FSYY\AppData\Local\Temp\claude\D--dev-Tinwave\53bf9027-c019-4dd4-82f1-5a72c6705b0c\scratchpad\r2-nextest.txt
  - C:\Users\FSYY\AppData\Local\Temp\claude\D--dev-Tinwave\53bf9027-c019-4dd4-82f1-5a72c6705b0c\scratchpad\r2-stress.txt
  - C:\Users\FSYY\AppData\Local\Temp\claude\D--dev-Tinwave\53bf9027-c019-4dd4-82f1-5a72c6705b0c\scratchpad\r2-deny.txt
