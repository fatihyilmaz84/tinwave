REVIEW T-0063 (round 2)
verdict: APPROVE
checks (run by reviewer): fmt pass (`cargo fmt --all --check`, exit 0). clippy pass (`--workspace --all-targets -D warnings`, exit 0). build pass (`cargo build --workspace`, exit 0). tests pass: nextest `--workspace --no-fail-fast` gave "802 tests run: 802 passed, 6 skipped", exit 0. deny pass (advisories, bans, licenses and sources ok; exit 0). Main has moved since the branch base 65b8bc3 (T-0066 changed engine.rs and tests/common/mod.rs). I tested the merge too: the `git merge-tree` of main 3af8332 and 4a862da is clean, and in a scratch copy of that merged tree nextest `-p tinwave-playback` gave "362 passed, 5 skipped" and clippy `-p tinwave-playback --all-targets -D warnings` exited 0.

acceptance criteria:
- [x] A 2 ms fade is caught by the relative click test; a 5 ms fade passes. Verified by: a scratch copy where I added an eprintln of the margin to `click_free`.
  - Unmutated, `no_click_at_a_rate_step_...` at 0.4 % passed every run, 69 in all:
    - 10 idle runs: always exactly -0.202 % (deterministic).
    - 59 runs under 16 CPU burners: from -0.255 % to +0.294 %, 0 restarts starved of 40 in every run.
  - With `RATE_FADE_MS` 5 -> 2 at engine.rs:28 (I confirmed the recompile), it failed 12 of 12 runs:
    - idle: +0.711 % four times;
    - under load: +0.674 % to +0.995 %.
  - The synthetic `the_click_check_passes_a_5_ms_fade_and_fails_a_2_ms_one` passes.
  - Margins: the passing side has about 0.1 percentage point of room (worst +0.29 % against 0.4 %). The failing side has about 0.27 points (best mutant +0.67 %). The test's own analysis caps a 5 ms fade at 0.3 % plus less than 0.1 % ripple, so the bound holds by design. It is thin, though, see non-blocking.
- [x] Rate restart reaches the first new audio without backoff delay (simulated device). Verified by: `decoder::tests::a_rate_flush_is_polled_at_the_minimum_for_a_while_and_then_backs_off`. It checks 30 polls at FLUSH_WAIT_MIN, then 2/4/8/16 ms up to 50 ms. A command's flush backs off from its first timeout, and a command resets to `AckPoll::new()`.
  - Mutation where the rate flush never backs off (condition without `&& fast_left > 0`, decrement made saturating): the test fails on its assertion (`[1ms,1ms,1ms,1ms]` vs `[2,4,8,16]`).
  - The builder's own mutation (condition removed, `-= 1` kept) is caught too, but only by a u32 underflow panic at decoder.rs:812, not by the assertion.
- [x] rate.rs has no real sleeps for the throttle and runs faster; stable under 16 parallel copies. Verified by: back-to-back runs of main's rate binary (scratch copy of main e1dfe9d; 3af8332 adds no crate changes) and the branch's, each with its own target dir.
  - Idle: main 1.55–1.58 s, branch 1.16–1.39 s (harness time). Per test under nextest: main 2.10 s total, branch 1.47 s.
  - Under 16 burners, wall time over two sets of 4 runs:
    - main: 3.3 / 5.6 / 3.3 / 52.7 and 4.9 / 3.1 / 3.0 / 3.6 s;
    - branch: 4.7 / 5.3 / 3.4 / 12.9 and 4.7 / 4.5 / 5.4 / 4.7 s.
  - So under load the branch's median is about 1 s higher, with 24 tests against main's 20. Its tail is much smaller than main's, and the round-1 24–40 s / 83 s cost is gone.
  - 16 parallel copies of the branch binary: 3 rounds (two with `--test-threads 4`, one with default threads), 48 of 48 copies passed, about 10 s per round.
  - The throttle waits are all `h.clock.advance`.
- [x] A track failing again on restart reports one TrackFailed and leaves as Failed. Verified by: `a_track_that_breaks_again_...` and `a_track_that_cannot_be_reopened_...` pass, unchanged since round 1 (the round-2 commit does not touch that code).
- [x] fmt, clippy, nextest, deny pass (see checks).

Round-1 blocking findings:
- Blocking 1 (unbounded 1 ms polling) is fixed. I reran the failed-switch-then-`set_rate(1.25)` scenario in a scratch copy, with a counter in `wait_for_ack` and a 300 ms wait after the failed switch: without it, the Reconfigure restart adopts the new rate and no flush happens.
  - Fixed code: 29 polls in the first 0.5 s (the 30 fast polls at the 15.6 ms timer), then 37 / 32 / 33 / 32 per 2 s, about 16/s. That stays under the documented "about 20 wakeups/s".
  - Never-back-off mutant: 127 / 130 / 127 / 129 per 2 s, the round-1 bug.
  - Resets: a command received in `wait_for_ack` resets the state (decoder.rs:803; unit-tested). The ack resets it (decoder.rs:375): the code is correct, but no test covers it (see non-blocking).
- Blocking 2 (cost of the change_rate pacing) is fixed. 256-frame callbacks, a 12,000-frame pre-buffer, no pacing sleep, phases varied with `(i*41)%109`, and the timings above.

Q4b (does the double edit guarantee the request is raised before the fast callbacks?): it makes it very likely; it does not guarantee it.
- The loop drains every queued command with `try_recv` before `service_rate` (decoder.rs:347-360).
- Suppose the decoder is preempted after emitting QueueEdited #1 but before its drain sees Empty, and the test thread sends edit #2 in that window. Edit #2 is then taken in the same drain, and its answer comes before the flush request is raised.
- If that happens, the 256-frame callbacks drain the ring with no sleep. The queue level always passes through 1..600 frames on the way down, so the restart is flagged `starved` and excluded. The test fails only if more than half of the restarts are starved.
- I saw 0 starved restarts in every run.

blocking findings:
- none

design check: n/a

non-blocking findings:
- crates/tinwave-playback/src/decoder.rs:375 — the ack reset `flush_wait = AckPoll::new()` now matters for the acceptance criterion: without it, `fast_left` leaks across rate flushes and a command's backoff carries into the next rate flush. Yet removing it passes every playback test (I ran the mutant: all lib and integration tests green). A unit or loop-level test that runs two rate flushes in a row, or a command flush and then a rate flush, would pin it.
- crates/tinwave-playback/tests/rate.rs:240-247 — the comment says the answer to the second edit means "the pass of the decoder's loop that raises the flush request is over". It is not a guarantee: see the Q4b race. The comment should say "almost always" and point to the `starved` exclusion that covers the rare miss.
- crates/tinwave-playback/tests/rate.rs:206-219 with 264 — under load the restart index is often recorded one 256-frame callback late. That happens when the decoder's flush increment and its first push both land between two test callbacks: `step` records `left.len()` after a callback that already played new-stream audio. Measured: restart 1 at 6496 instead of 6240, and recordings of 262–265k frames instead of 257,605.
  - It is harmless for coverage: the shift (256) is well inside RESTART_WINDOW (1440), so the fade stays in the window.
  - It is also why the passing margin varies under load (-0.26..+0.29 %) but sits fixed at -0.202 % when idle.
  - With about 0.1 point of room at 0.4 %, a rare false failure on a busy machine cannot be ruled out. I saw none in 69 runs plus 48 parallel copies.
- crates/tinwave-playback/src/decoder.rs:808 — the overflow-panic mutation shows that `fast_left -= 1` relies on the guard. `saturating_sub` would be more robust, but the code is correct as written.
- crates/tinwave-playback/src/decoder.rs:48 — this module doc line is longer than the 100-column style (rustfmt does not wrap comments).
- Scope: only decoder.rs and tests/rate.rs changed in both commits; no work/ edits; the worktree is clean.

proposals:
- P: Pin the ack reset of the flush poll state | why: removing decoder.rs:375 passes every test, but it is what keeps rate restarts on fast polling | size: S
- P: Record the rate restart index before the callback that plays new audio | why: it removes the load-dependent margin variation in the click test (`step` reads `flushes` after the callback) | size: S

notes for lead:
- The branch is based on 65b8bc3, behind main's T-0066 merge. The merge is clean, and the merged tree's playback tests and clippy pass. A rebase or merge before landing is still worth a final gate run.
- All my scratch copies (scratchpad\rv63r2: br, main, merged, each with its own target dir) are deleted. My burners stopped (none left). None of my processes are running. The cargo-nextest that was running at the start belongs to another agent.
