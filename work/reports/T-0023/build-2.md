HANDOFF T-0023 (round 2)
status: done
branch: t/T-0023-live-queue-edits   commit: ffe9748 (on top of 57aca71 and b9eb089)
summary: The blocking finding and the two extras from round 2 are fixed. A queue edit that names the playing head in `QueueEdit::Remove` is now rejected before anything is sent. P-0198 is done: the decoder tracks the last item it opened, so an edit on the still-audible last track works when the items after it failed to open. The `LoadRequest.leaving` doc line is added.

**Round 2 items**
- **Blocking: Remove naming the head.**
  - `Controller::edit_queue` returns `InvalidArgument("the playing track cannot be removed by a queue edit")` when a Remove item equals `head`. Nothing is sent and nothing is recorded.
  - `an_edit_that_contradicts_its_own_description_is_rejected_before_it_is_sent` now has two extra cases: `Remove{[1]}` and `Remove{[3,1]}`.
  - Mutation: with the check weakened, that test fails.
- **P-0198: last opened index.**
  - The decoder has a new `last_opened` field. It is set when an item's mark is published and cleared in `reset_stream`.
  - Once the queue is exhausted, the edit base is `last_opened` instead of `cur - 1`.
  - The first version of the test exposed a second bug. After the failed successors, `cur` was past the end of the list, so the new tail was never opened. The edit now sets `st.cur = base + 1` when no reader or flush is pending.
  - New test: `an_edit_behind_a_track_whose_successors_all_failed_to_open_is_applied`. The queue is [A, missing, missing]. The test waits for both TrackFailed events, edits behind A, and checks that nothing was refused, that A is followed bit-exactly by the new item, and that items [1,4] started. Mutation: reverting to `cur - 1` fails it.
- **Docs.** `LoadRequest.leaving` now says that after `QueueEnded` nothing is current, so a `Some` makes next/previous a no-op, and the shell passes `None` then.

**checks**
- fmt pass, clippy `--workspace --all-targets -D warnings` pass, deny pass (advisories, bans, licenses, sources ok).
- Tests: three full `cargo nextest run --workspace` runs, each 506 passed / 0 failed / 6 skipped. No storage_growth timeout this time.
- Stress: `--stress-count 15 -j 24` over queue_edits, controller_edits, controller_random, pairing, rt_alloc and controller_rt_alloc passed 15 of 15.
- Tooling: I used no `python3`. The targeted tests were run before the full gates, once at the end.

files changed (this commit): crates/tinwave-playback/src/decoder.rs, crates/tinwave-playback/src/controller/{mod,types}.rs, crates/tinwave-playback/tests/{queue_edits,controller_edits}.rs

signals added/changed: none this round. No new kinds; emission is as in the earlier rounds.

ui: none

docs updated: the `LoadRequest.leaving` doc only.

deviations from ticket: none beyond rounds 1 and 1-updated.

proposals:
- None new. The non-blocking reviewer item about Remove or Reorder of items at or before the head waits on the parked queue mirror (P-0162).

questions for lead: none
