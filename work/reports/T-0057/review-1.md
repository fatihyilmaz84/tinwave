REVIEW T-0057 (round 1)
verdict: CHANGES_REQUESTED
checks (run by reviewer): fmt pass (worktree and merged tree); clippy `--workspace --all-targets -D warnings` pass (worktree), and `-p tinwave-playback -p tinwave-db --all-targets -D warnings` pass on the merged tree; tests: nextest on the worktree gave 748 passed, 6 skipped (ignored real-device tests, not run), and the merged tree (playback and db) gave 431 passed, 4 skipped. Stress: 8 copies x 4 rounds of exclusive_reopen, controller_exclusive, controller_thread, rt_alloc and the lib unit tests gave 160/160 ok. Under heavier load, exclusive_reopen failed 1 of 32 runs (48 test processes at once) and 3 of 120 runs (40 copies at once), always the same assertion (see blocking finding). deny pass (advisories, bans, licenses, sources ok).

Merge with main dd0826a: `git merge-tree` is clean (tree 9647e35). Main's changes since 7a6cf5d are only in crates/tinwave-db, docs/ and work/. No workspace crate depends on tinwave-playback or tinwave-db, and Cargo.lock is untouched on both sides. The merged tree was built and tested in a scratch git-archive copy with its own CARGO_TARGET_DIR. That copy has been deleted.

acceptance criteria:
- [x] Exclusive, 44.1 k → 96 k: the output reopens at 96 k at the boundary, both tracks bit-exact, no callback allocation — verified by:
  - `exclusive_reopen::a_rate_change_reopens_the_output_at_the_file_rate_and_both_tracks_are_bit_exact`: opens are [48000, 44100, 96000] and the audio equals the 24-bit references exactly. The reopen happens with wall == 44100 and at least 480 frames of silence, so the tail is not cut.
  - `a_24_bit_pair_reaches_the_device_as_the_files_integers`: Pump to I24In32, no dither, which matches the convert.rs policy that exact integers get no dither.
  - `rt_alloc::render_never_allocates_across_exclusive_reopens` (opens [48k, 44.1k, 96k, 44.1k, 96k], 0 allocations).
  - Code reading: renderer.rs:107-114 only adds relaxed atomics. All ring, Renderer and stream construction happens in `Engine::open_stream` on the control thread. The `refused` and `device_rates` mutexes are never touched by the callback.
- [x] Device refuses the rate: falls back to shared + resampler, reported once — verified by:
  - `a_refused_rate_plays_resampled_at_the_engine_rate_and_is_reported_once`: opens [48k, 44.1k, 96k, 48k, 44.1k, 48k], one fallback info, 96 k not retried.
  - `a_device_that_refuses_exclusive_altogether_falls_back_to_shared_once`.
  - `engine::tests::refused_rates_are_remembered_per_device...`: the refused set is reset on switch_device.
  - Code reading: the `open_stream` loop runs at most two passes, because the second is at `sample_rate`, where the refusal branch cannot fire. If every rate is refused, `follow` becomes false and parking stops, so no reopen loop is possible.
- [x] Signals per the catalog (no new kinds); fmt, clippy, tests, deny pass — verified by:
  - The lead ruling is applied at mapper.rs:983-992.
  - `controller_exclusive` (3 tests): rate reopen gives 0 toggles; refusal gives 0 toggles with 3/3 completed; the listener's off and on give exactly one toggle each.
  - `controller_signals::exclusive_mode_toggled_follows_the_stream_info...` was updated.
  - Gates above. The acceptance criteria hold; the change request is for test determinism only.

blocking findings:
- crates/tinwave-playback/tests/exclusive_reopen.rs:712-716 — nondeterministic assertion `h.engine.rate_stats().flushes >= 1` in `a_speed_change_during_the_second_file_restarts_inside_its_own_stream`.
  - Failure scenario: after `set_rate(1.5)` the test calls `play_to_end`, which renders whenever 480 frames are buffered. The decoder has usually pushed all of the 96 kHz file already. If its thread is not scheduled before the test thread drains the ring, `Decoder::service_rate` finds nothing audible and adopts the rate without a flush (correct engine behaviour, decoder.rs:712-715), so `flushes == 0`.
  - Reproduced 4 times under parallel load (1/32 with 48 concurrent test processes, 3/120 with 40 concurrent copies), always this assertion. It contradicts the handoff's claim that tests wait on conditions.
  - Suggested fix: after `set_rate`, wait (bounded spin) until `rate_stats().flushes >= 1` before rendering on, while the ring still holds audio. Calling `h.out.render(&mut [], 0)` plus `poll()` acknowledges the flush without draining audio, because an empty callback cuts the fade. Then the flush is guaranteed and the rest of the test is unchanged. Re-run the parallel stress (40 copies) afterwards.

design check: n/a

non-blocking findings:
- crates/tinwave-playback/src/engine.rs:794 (used by controller/runtime.rs:273-277) — the 10 ms fast poll is not bounded in time.
  - `reopen_pending()` is true for the whole drain wait of a boundary park. The decoder parks up to about 1 s (the ring) before the boundary, and while paused neither `consumed` nor `idle_frames` advance.
  - So a pause in the last second before a rate change keeps the controller thread waking every 10 ms for the whole pause. A scratch probe confirmed it: pending stays true across 2,000 paused callbacks and polls.
  - Similarly, `command_pending()` stays true indefinitely when a load is stuck: either the reopen failed (OutputLost, decoder parked with a gen), or the output is lost before a flush ack. Before this ticket that state polled at 100 ms.
  - It is not a busy loop, only wasted wakeups. Cheap fix: only report a drain-waiting park as pending while not paused, and don't fast-poll after OutputLost. Recommend doing it in round 2.
- crates/tinwave-playback/src/decoder.rs:615-636 with 497-517 — a `replace_after` that lands during a boundary park (the last ~1 s of a track whose successor has another rate) does not cancel the park.
  - Scratch probe: queue [a 44.1k, b 96k], decoder parked for b, "play next" with c 44.1k. Opens were [48k, 44.1k, 96k, 44.1k], i.e. a useless 96 kHz open and two reopens.
  - About 510 frames of silence fell between two same-rate tracks, which `replace_after` documents as gapless. No audio was lost; on a real device this is two exclusive opens.
  - The fix needs care: a Resume for a withdrawn park is currently dropped with "nobody to feed it".
- crates/tinwave-playback/src/controller/mapper.rs:984-991 — the fallback branch sets `reported_mode = info.mode` unconditionally, which can erase a deferred listener toggle.
  - Scenario: outside a session, the listener turns exclusive on (OutputReopened, no fallback, deferred). Then an append to the ended queue reopens at a refused rate (fallback, still Exclusive). The deferred `on:true` is never reported.
  - Narrow; fits with T-0067.
- crates/tinwave-playback/src/engine.rs:853 — `Cmd::Resume` goes through `try_send`. A full command queue (64) turns a transient Busy into OutputLost, and the parked decoder then waits for a restarting command. Unlikely, because a parked decoder drains its queue; worth a comment or a retry on the next poll.
- No test for switch_device during a boundary park. A scratch probe showed it is correct: opens [48k, 44.1k, dac 44.1k, dac 96k], the tail of A from the heard frame plus B bit-exact, TrackLeft 24000/48000 Finished. Worth adding as a regression test.

proposals:
- P: Re-evaluate a boundary park on a queue edit | why: a "play next" in the last second before a rate change opens a stream for a track that no longer plays and breaks same-rate gapless | size: S
- P: Regression test for switch_device during a boundary park | why: the path is correct today (probe) but untested | size: S

notes for lead:
- Only the flaky test blocks. The engine logic held up under review:
  - Park/resume handshake: generation guard (reopen_gen read before the request, unpark before the reopen_gen store); Release/Acquire on park total/drain → reopen.
  - idle_frames: reset to 0 is sequenced before the consumed Release, so there is no stale drain.
  - Load, seek and switch during a park, refusal path, Mark out_rate/timeline_rate.
- Round 2 should also fix the fast-poll-while-paused issue.
- Docs listed in build-1b (ADR 0005 decision 5 and "Not verified", catalog `ExclusiveModeToggled` row, architecture.md, PO-RUNBOOK) remain lead work on merge.
- Nothing left running. Two older processes are not mine and were left alone: a python.exe from 2 Nov (cmake-build) and the T-0065 builder's cargo, which had already exited.
- Scratch folder C:\Users\FSYY\AppData\Local\Temp\claude\D--dev-Tinwave\53bf9027-c019-4dd4-82f1-5a72c6705b0c\scratchpad\rev57 (stress logs, merged copy, its target dir, probe tests) has been deleted.
