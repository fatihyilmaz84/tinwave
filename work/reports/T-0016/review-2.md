REVIEW T-0016 (round 2)
verdict: CHANGES_REQUESTED
checks (run by reviewer): fmt pass; clippy `--workspace --all-targets -D warnings` pass; tests pass (`cargo test --workspace` 216 passed / 0 failed; tinwave-playback 10× debug and 8× release together with tinwave-test-alloc, all clean); deny pass; **stress: 1 failure in 528 concurrent processes** (blocking 3)

The fixes hold, but the leave-pairing invariant this round introduced breaks in two reproducible paths, and one new test can hang under load.

Lead's 5 checks:
1. Stress, same shape as round 1: 336 processes (7 binaries × 6 copies × 8 rounds) ⇒ 1 failure; 192 more (failures/gapless/seek/engine × 8 × 6) ⇒ 0. Total 1 in 528 (round 1: 4 in 336). Round-1 races gone; the one failure is in the new failures.rs (blocking 3).
2. CorruptPackets mutation: `self.drop_reader(st)` → `st.reader = None` at decoder.rs:209/216/250 (Load, Seek, Reconfigure) fails exactly the three skip/seek/switch tests; natural end still passes via `finish_item`. Round-1 blocking 2 resolved.
3. rt_alloc and tinwave-test-alloc: a Vec in render's play path fails with 748, in the flush branch with 8; `count_allocations` counts only the calling thread (thread-local `IN_REGION`/`COUNT`; `other_threads_are_not_counted`); alloc, alloc_zeroed, realloc, dealloc counted; crate dev-only (`cargo tree -i tinwave-test-alloc -e normal` empty); tinwave-playback back on workspace lints with no `unsafe`; test-alloc denies `unsafe_code` with one `#[allow]` + SAFETY comment; release `counter_detects_allocation_in_a_callback_shaped_region` passes.
4. Leave pairing: two violations reproduced (blocking 1, 2); no other zero-or-two cases found in load/load, load-before-applied, seek-fails, open-fails, mid-decode-fail, Append-after-end, stop interleavings.
5. Scope: crates/tinwave-playback/, crates/tinwave-test-alloc/ (lead-approved), fixtures/audio/, Cargo.toml [workspace.dependencies], Cargo.lock, deny.toml.

acceptance criteria:
- [x] 0-frame gap (WAV, FLAC, ALAC, MP3+LAME) — gapless.rs, lossless `assert_eq!(err, 0.0)`.
- [x] Seek sample-exact — seek.rs (strict 0.0) and failures.rs `seek_to_the_last_frame_works_in_every_container`.
- [x] Zero allocations/frees in the render callback — rt_alloc.rs + both mutations.
- [x] Position marks monotonic at 10 Hz — position.rs.
- [x] No decoder/file IO on the render thread — blocking_reader.rs + code review (renderer.rs unchanged since round 1).
- [ ] fmt, clippy, tests, deny pass — pass in normal runs, but failures.rs has a load-dependent hang (blocking 3).

blocking findings:
1. **Seek after QueueEnded gives a second `TrackLeft`** — engine.rs:263-279 (`seek`). After the queue ends, `position()` still resolves to the retained last mark, so `seek` is accepted; `cursor` is taken and no `TrackStarted` is emitted; the restarted stream's Initial mark matches `announced` and is adopted silently; at its end `TrackLeft{Finished}` and `QueueEnded` are emitted again. Repro: load B at 40000, play to end, `seek(1000)`, play to end ⇒ `[TrackStarted{2}, TrackLeft{2,Finished}, QueueEnded, Seeked{2,..}, TrackLeft{2,Finished}, QueueEnded]`. Real-world: dragging the seek bar after the last track ends ⇒ a second Completed with no PlayStarted. Fix: reject `seek` with `NothingLoaded` once the queue has ended, or treat it as a restart (emit `TrackStarted{item, frame, Requested}` before `Seeked` when `cursor` and `announced` are both None). Add the scenario with `assert_paired`.
2. **A failed `switch_device` produces a duplicate `TrackStarted`** — engine.rs:400-404 (error branch). `Reconfigure` was already sent, so the decoder restarts and pushes a new Initial mark; `command_sent` isn't called on this path, so `announced` is None while `cursor` holds the old mark; the next `poll` emits `TrackStarted{item, Advance}` while the original is still open. Repro (latency 0): play 4800, `fail_next_open`, failed switch, poll, retry ⇒ `[TrackStarted{1,0,Requested}, OutputLost, TrackStarted{1,4800,Advance}, OutputReopened, ...]`. Fix: on the error path still record the sent command (`command_sent(gen, resume item, frame, true)`); add the repro as a test.
3. **Load-dependent hang in `a_track_failing_mid_decode_is_left_as_failed_not_finished`** — failures.rs:229. The loop calls `wait_buffered(max(1, ...))` before every block; if the test thread consumes B's last frames and polls before the decoder sets `eos`, `TrackLeft{2}` isn't emitted yet and the next `wait_buffered(1)` waits for audio that never comes ("decoder did not deliver 1 frames in time"). Hit in stress (stress-r2/4-2-failures-*.log). Fix: when the ring is empty, render and poll in a bounded loop as `Harness::finish` does; check sibling loops.

design check: n/a

non-blocking findings:
- engine.rs:373-375: `primed = false` stored after the old stream is dropped; the decoder can push into the old producer and set `primed` before handling `Reconfigure`, so a spurious `Underrun` remains possible in a narrow window. A per-renderer "has seen data since open" flag would close it.
- reader.rs (`length = valid.or(container_frames)`): for VBR MP3 without Xing/Info, Symphonia's `num_frames` can be an estimate; seeks near the end could be wrongly rejected/allowed. Untested (fixtures are CBR with a tag).
- Every mid-decode/open failure is now `LeaveCause::Failed`; T-0024 must map it to neither Completed nor Skipped.

proposals:
- P: Property-style test driving random load/seek/stop/switch/fail-open/append sequences through the engine, checking `assert_paired` after each step | why: both pairing bugs came from interleavings no hand-written test covered | size: M

notes for lead:
- All three blocking items are small (a few lines in engine.rs plus a test loop fix). Round-1 blocking 1 and 2 resolved; lead items 3-9 implemented as described.
- Repro tests (uncommitted): session scratchpad mut2/crates/tinwave-playback/tests/review_r2.rs (git archive copy of b41cf11).
- Stress logs: session scratchpad stress-r2/, stress-r2b/.
- Worktree not modified apart from target/.
