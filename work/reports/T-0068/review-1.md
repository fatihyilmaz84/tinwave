REVIEW T-0068 (round 1)
verdict: CHANGES_REQUESTED
checks (run by reviewer): fmt pass (exit 0), clippy pass (`--workspace --all-targets -D warnings`, exit 0), tests pass (nextest: "835 tests run: 835 passed, 7 skipped", exit 0; plus `nextest run -p tinwave-playback --stress-count 8`: "8/8 stress run iterations: 8 passed"), deny pass (advisories ok, bans ok, licenses ok, sources ok), `cargo build --workspace` pass (exit 0). rt_alloc, output_rt_alloc and controller_rt_alloc all pass.

acceptance criteria:
- [x] [a 44.1k, b 96k] parked for b, play-next c 44.1k: no 96 kHz open, a→c gapless (bit-exact, no silence). Verified by `exclusive_reopen::a_play_next_during_a_boundary_park_keeps_a_same_rate_hand_off_gapless_and_opens_no_stream`, which asserts rates `[48000, 44100]`, a++c bit for bit and 0 underruns. I re-ran the builder's mutation (drop `reconsider_park` in `edit_queue`): 2 decoder unit tests and 3 integration tests failed. I also removed that test's `!reopen_pending()` assert under the same mutation, and `render_exact` still failed on its own ("decoder did not deliver 441 frames in time"), so the gapless check is real.
- [x] Edit to another rate re-parks for that rate. Verified by `an_edit_to_a_track_of_another_rate_parks_again_for_that_rate`: opens `[48000, 44100, 88200]`, a++d bit-exact, old stream played out first (wall 22050, silence ≥ 480). It fails under the mutation above.
- [x] Stable under parallel stress; fmt, clippy, nextest, deny pass. Verified by my own runs (above), on a machine already loaded by two other builders.
- [ ] Scope item P-0294 ("make the position follow what is heard"; the handoff and the rustdoc at engine.rs:564-566 say "never back"). Problem: the position goes back when a queue edit withdraws the park during the drain. See the blocking finding.

blocking findings:
- crates/tinwave-playback/src/engine.rs:1138-1151 (`view`), triggered by decoder.rs:706-718 (`reconsider_park`).
  - Defect: the drain credit only counts while a request is published (`requested != 0`). When an edit withdraws the park while the device is playing out its buffered tail, `requested` becomes 0 and the credit disappears. Nothing raises `heard_floor` on a withdrawal; it is set only when the engine claims the request. So `shown` falls back from `played + credit` to `played`.
  - Scenario: exclusive mode, latency 480, a (44.1k, 0.5 s) then b (96k). a is fully consumed, then 2 empty 100-frame callbacks, then `replace_after(1, [c 44.1k])`. `position()` goes from frame 23695 to 23478: back about 217 timeline frames. In general it goes back by up to the device latency.
  - Reproduced with this reviewer test in a scratch copy (it fails on the branch as is):
    ```
    wait_parked; render a to 22_050 consumed; 2× render(100) == 0 + poll;
    before = position(); replace_after(1, [c]); expect_edit_applied; after = position();
    assert!((before.item, before.frame) <= (after.item, after.frame))
    ```
  - This is a regression this branch introduces: main has no credit, so nothing can drop.
  - PositionMarks are not affected, because `last_grid` is monotonic. `position()` is, and it feeds the UI and the position carried by Paused/Seek/Volume signals.
  - The window is about one device latency plus one poll interval per boundary.
  - Fix (S): keep a monotonic floor of the drain position, so a withdrawn park freezes the position instead of dropping it.
    - Option 1: make `heard_floor` a `Cell<u64>` and raise it to `shown` in `view()` whenever a drain credit is applied.
    - Option 2: record the last drain `shown` and lift `heard_floor` to it when the request disappears without the engine's claim.
    - Either way, the floor must also cover `position()` reads between polls.
  - Add the test above, with a same-rate edit and a remove variant.

design check: n/a (no UI)

non-blocking findings:
- crates/tinwave-playback/src/engine.rs:915: no test covers the engine's lost-claim branch.
  - My mutation `if !take_reopen(..) && id == u32::MAX` (always proceed) survived every lib and exclusive_reopen test.
  - `a_request_the_decoder_withdrew_is_not_answered` returns early at `rate == 0` and never reaches the CAS.
  - The window (between `reopen_request()` and the CAS) is a few instructions and hard to drive deterministically, so I'm noting it, not blocking.
  - The protocol itself is sound:
    - Only the decoder stores the slot. The engine only CASes it. Ids are fresh per park, so no ABA.
    - `park()` stores total and drain before the slot (Release), the engine reads the slot first (Acquire), and the CAS is AcqRel. So `total` and `heard_floor` cannot come from a stale park.
- crates/tinwave-playback/src/decoder.rs:1213 together with 706-718: with a refused rate (resampled stream), the resampler of the head is flushed (`finish`) when the park is requested.
  - After a withdrawal, a successor at the head's file rate gets a fresh resampler instead of sharing the old one, so the join is a resampler finish plus a new prime, not the usual shared-resampler hand-off.
  - Sample count is kept and the path is not bit-exact anyway. It is only reachable with refused rates.
- My mutation on the decoder's lost-CAS branch (drop the park even when the CAS lost) is caught by `a_queue_edit_after_the_engine_took_the_request_leaves_the_decoder_parked_for_the_resume`. Good.
- Interleavings I walked, all correct (no orphan `Resume`, no double open, no stuck park, no stream left without audio):
  - Edit before the claim: the decoder withdraws and continues, or re-parks with a new id.
  - Claim before or during the edit: the decoder's CAS fails, it stays parked, and the `Resume` opens the edited `cur`, re-parking non-boundary if its rate differs. One wasted open, as the builder documented.
  - Two edits in a row, and an edit while re-parked: each withdrawal CASes its own id.
  - Unpublished `park_req`: dropped, the tail still pushes, then `cur` = base+1.
  - Device switch while parked: `command_taken` unparks and the Reconfigure restarts.
  - OutputLost while parked: `service_reopen` returns while `stream` is None. If the open fails after a claim, the decoder stays parked until a switch_device; that already happens on main.
- `QueueEdited` is emitted after the withdrawal, so the new `!reopen_pending()` asserts at exclusive_reopen.rs:929 (same rate) and :1015 (remove: the queue ends, no re-park) are deterministic.
- Only the other-rate test needed `wait_parked`, and the builder fixed it.
- The older asserts at :459, :469 and :501 follow wait loops or a `settle_polling` after the load unparked; they are not racy.
- Real-time safety: renderer.rs is unchanged. The new work is atomic loads in `view()` on the control thread.
- Signals: marks during the drain go through the same 10 Hz grid, with `last_grid` monotonic and `grid_base` reset when the next track is announced. That adds at most about latency/100 ms marks per boundary. No new kind, so this is not a new signal under signal-catalog.md §8.

proposals:
- P: Integration test for the claim-first interleaving (the engine claims between the decoder's receipt of the edit and its CAS) | why: covered only by a unit test today, and the engine's lost-claim branch has no coverage at all | size: S
- P: Hold the engine's claim back while a `replace_after` is unanswered (the builder's proposal) | why: avoids the one wasted open plus gap when an edit loses the race | size: S

notes for lead:
- main has moved to ca64629, with work/ commits only since fba8f71. `git merge-tree` against main is clean.
- The shared.rs change is limited to `take_reopen` and its test, as you allowed.
- My scratch copy and its target dir have been deleted. The worktree is untouched (`git status` clean). Nothing I started is still running. The `cargo` and `python` processes I saw were already running when I began.
- Gate logs: C:\Users\FSYY\AppData\Local\Temp\claude\D--dev-Tinwave\53bf9027-c019-4dd4-82f1-5a72c6705b0c\scratchpad\nextest.txt, C:\Users\FSYY\AppData\Local\Temp\claude\D--dev-Tinwave\53bf9027-c019-4dd4-82f1-5a72c6705b0c\scratchpad\deny.txt and C:\Users\FSYY\AppData\Local\Temp\claude\D--dev-Tinwave\53bf9027-c019-4dd4-82f1-5a72c6705b0c\scratchpad\stress.txt.
