HANDOFF T-0030 (round 1)
status: done
branch: t/T-0030-engine-append-and-docs   commit: a6e90ab (on top of main c0bfd2a)
summary: All four items done in crates/tinwave-playback only. Items 1–3 each have a test that fails with its fix reverted; item 4 is docs.

acceptance criteria:
- [x] Each item has a failing-before test (each fix reverted alone; named test fails, rest unaffected):
  1. `waiting_for_an_appended_track_to_open_is_not_an_underrun` (tests/append.rs) and unit `a_revived_queue_is_buffering_until_its_first_push` fail with the renderer's `buffering` check removed.
  2. `switch_while_an_appended_track_is_still_opening_does_not_drop_it` fails (B never plays, 15 s timeout) with the decoder's `Reconfigure{item: None}` reset back to `cur = queue.len()`; directed and deterministic (gated opener holds the decoder in B's open while the switch is queued).
  3. `switch_after_an_appended_track_is_decoded_but_not_heard_does_not_drop_it` fails with the unheard-mark lookup removed from `switch_device` (latency 960: B fully decoded, not heard, `cursor` and `announced` None).
- [x] Property test (seeds 1..32) passes; rt_alloc passes (Vec in render ⇒ 772).
- [x] fmt, clippy `-D warnings`, tests, deny pass.

changes:
1. New `Shared.buffering` atomic: decoder sets it before clearing `eos` when `Append` revives an ended queue; clears it after its next push, on `finish_queue`, in `reset_stream`. Renderer reads `eos` then `buffering` (Acquire) and counts an underrun only when `!eos && !buffering && seen_data`. Chosen over a stream epoch (no extra renderer state, only the revival path).
2. Directed regression test for `Reconfigure{item: None}` (above).
3. `switch_device` with no current track: resumes the first mark with seq above `seen_seq`, from its offset, emitting `TrackStarted{Advance}` before the command; otherwise unchanged; test asserts exactly one TrackStarted and `assert_paired`.
4. event.rs docs: `Seeked` only while a track is current (`Engine::seek` rejected and silent otherwise); `Underrun` only after audio has flowed since open/flush (waiting for first audio after load, seek, switch or append-to-ended is buffering, no event).
tests: new tests/append.rs (3) + renderer unit test; `Gate`, `GatedSource`, `GatedOpener` moved to tests/common with an `only: Option<PathBuf>` filter.
checks: fmt pass, clippy pass, tests 231 passed / 0 failed (workspace), deny pass.
stress: 10 playback test executables concurrent. First 540-process run had 1 failure in its own new test (asserted zero underruns before the append; `finish`'s callbacks can legitimately count one while the decoder notices A's end) — test now baselines after `QueueEnded`. After the fix: 540 (6 × 9) + 600 (12 × 5) = 1140 processes, 0 failures.
signals added/changed: none; deviations: none; proposals: none; questions: none
