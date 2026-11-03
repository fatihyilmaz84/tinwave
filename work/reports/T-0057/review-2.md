REVIEW T-0057 (round 2)
verdict: CHANGES_REQUESTED
checks (run by reviewer, worktree @ f743811):
- fmt: pass.
- clippy (`--workspace --all-targets -D warnings`): pass.
- tests: nextest gave 767 passed, 6 skipped (ignored real-device tests, not run).
- deny: pass (advisories, bans, licenses, sources ok).
- Merge with current main 8ee670e: `git merge-tree` is clean (tree d848535). It differs from the branch only in work/ files, so the merged tree's code is the branch's code and the gates above apply to it.

stress (40 copies, 3 rounds each):
- exclusive_reopen: 120/120 ok. The round-1 flake (3/120) is gone.
- rt_alloc: 120/120 ok.
- lib unit tests: 120/120 ok.
- controller_exclusive: 119/120. The one failure is an iteration-budget timeout, see non-blocking.
- Mixed run (15 copies of each of the 4 binaries at once, 2 rounds): 120/120 ok.

controller_thread claim (T-0069) confirmed:
- Under the same load (40 exclusive_reopen and 30 rt_alloc copies alongside), baseline main failed 1/60 (`quitting_does_not_wait_for_a_decoder_stuck_in_a_read`, controller_thread.rs:433).
- The branch was 0/60 under that load.
- So the load flakiness predates this ticket.

acceptance criteria:
- [x] Exclusive, 44.1 k → 96 k: reopens at 96 k at the boundary, both tracks bit-exact, no callback allocation. Verified by `exclusive_reopen::a_rate_change_reopens_the_output_at_the_file_rate_and_both_tracks_are_bit_exact`, `a_24_bit_pair_reaches_the_device_as_the_files_integers` and `rt_alloc::render_never_allocates_across_exclusive_reopens`. These are unchanged since round 1; the render path was not touched this round.
- [x] Device refuses the rate: falls back to shared + resampler, reported once. Verified by `a_refused_rate_plays_resampled_at_the_engine_rate_and_is_reported_once`, `a_device_that_refuses_exclusive_altogether_falls_back_to_shared_once` and the engine refused-set tests (unchanged).
- [x] Signals per the catalog (no new kinds); fmt, clippy, tests, deny pass. Verified by `controller_exclusive` (3 tests), `controller_signals` and the gates above.

round 2 items:
1. Flaky test: fixed (exclusive_reopen.rs:775-790). After `set_rate`, the test spins (bounded, 30,000 x 100 µs) on empty callbacks plus poll until `flushes >= 1`. That holds whatever the scheduling, because an empty callback acknowledges the flush without draining audio. 0/120 at 40 copies.
2. Bounded fast poll: verified.
   - `reopen_pending()` (engine.rs:804-811) is false only for a drain-waiting park while paused. No poll can advance that state: the renderer adds no `idle_frames` and no `consumed` while paused.
   - `poll_soon()` (engine.rs:818-822) is false only when the stream is gone or the loss has been reported. Every other pending state still fast-polls: a load or seek park, an unsent resume (including while paused), and a pending command.
   - The only reopen that waits on the slow interval: the drain finished just before the pause, so up to 100 ms is lost while paused, which is inaudible.
   - After OutputLost a pending park is still served on the normal 100 ms poll, so nothing is stranded.
3. Resume on a full queue: no deadlock.
   - A parked decoder blocks in `wait_for_command`, so it drains the queue. Each poll retries the stash first (engine.rs:852), and fast-polls while it does. Disconnected gives OutputLost.
   - load, seek, stop, append and replace_after return `Busy` from `send` (engine.rs:1001) before any side effect, and the controller passes `Busy` to its caller as before.
   - `set_rate` and shutdown use `try_send` directly. They are only wake-ups, so they cannot misorder anything.
   - `switch_device` is the exception, see the blocking finding.
4. Regression test `a_device_switch_while_the_decoder_waits_at_a_boundary_resumes_and_reopens_on_the_new_device`: present and passing. It matches my round-1 probe: opens [(None,48k), (None,44.1k), (dac,44.1k), (dac,96k)], bit-exact, TrackLeft 24000±1 and 48000.

blocking findings:
- crates/tinwave-playback/src/engine.rs:602-650 together with 852 / 898 — `switch_device` while a Resume is stashed (`unsent_resume`) closes the working stream, then fails, and the next poll reports a reopen for a stream that no longer exists.
  - Failure scenario, reproduced with a scratch unit test in a git-archive copy (same setup as `a_full_command_queue_delays_the_resume...`, then `switch_device(Some("dac"))`):
    - `switch_device` calls `poll()`, the queue is still full, and it sets `self.stream = None` (line 625), dropping the reopened 96 kHz stream.
    - It opens dac at 96 kHz, then `send(Reconfigure)` gets `Busy` from the stash guard. It drops the dac stream, emits `OutputLost` and returns `Err(Busy)`.
    - On the next poll, `deliver_resume` sends the stale `Resume`, whose producer feeds a ring whose consumer was dropped. It emits `OutputReopened { device: None, info: Exclusive }` although `engine.stream` is None.
    - Observed: `switch=Some(Busy) stream=false events=[OutputLost]`, then `sent=Ok("Resume") events=[OutputReopened{..}] stream=false`, opens [(None,48000), (None,96000), (Some("dac"),96000)].
  - Result: the host sees "lost" followed by "reopened" and may stop recovering. The decoder resumes into a dead ring, so playback is silent until the next restarting command.
  - This is new in round 2: round 1 reported OutputLost honestly here. It is rare, since it needs 64 unread commands while parked, but it is the exact path item 3 exists for, and the result is an inconsistent engine state.
  - Fix (a few lines): at the top of `switch_device`, after its `poll()`, return `Err(PlaybackError::Busy)` while `unsent_resume.is_some()`, before touching the stream. Optionally, `service_reopen` can also drop a stash whose stream is gone. Add one unit test next to `a_full_command_queue_delays_the_resume...`. This is small enough for the lead to apply directly instead of a third builder round.

design check: n/a

non-blocking findings:
- crates/tinwave-playback/tests/controller_support/mod.rs:290-300 (`run_until`, 30,000 iterations with 100 µs sleeps) — `controller_exclusive::losing_exclusive_mode_to_a_refusing_device...` failed 1 of 120 runs at 40 copies with "never happened: the queue ended".
  - The failing copy took 28.2 s, the slowest of its round; its siblings that passed took 26-28 s. So this is the iteration budget running out under oversubscription (the resampled 96 kHz track is the most decode-heavy), not a hang.
  - Same class as T-0069; fold it in there (raise the budget or wait on buffered audio).
- Carried from round 1, scoped out by the lead: `replace_after` during a boundary park (T-0068), and the mapper fallback branch erasing a deferred toggle (T-0067).

proposals:
- P: Fold `controller_support::Rig::run_until` budget hardening into T-0069 | why: 1/120 timeout of a T-0057 test under 40-copy load, same pattern as controller_thread | size: S

notes for lead:
- The only blocker is the `switch_device`-during-stash path: a few-line guard plus one unit test, suitable for a lead takeover.
- Everything else in round 2 checks out. I re-checked item 2's poll paths and item 3's ordering and Busy handling by reading the code, and the stress results are above.
- Docs from build-1b and build-2 remain your merge work.
- Nothing of mine is left running. The cargo process still running (`cargo test -p tinwave-db --test storage_growth`, started 16:57) belongs to the T-0065 builder and was left alone.
- My scratch folder C:\Users\FSYY\AppData\Local\Temp\claude\D--dev-Tinwave\53bf9027-c019-4dd4-82f1-5a72c6705b0c\scratchpad\rev57b (branch copy with the probe, baseline main copy, their shared scratch target, stress logs) has been deleted.
