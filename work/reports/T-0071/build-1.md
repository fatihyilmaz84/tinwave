HANDOFF T-0071 (round 1)
status: done
branch: t/T-0071-cancel-storm-flake-again   commit: f194c81
summary: The flake was the fixed exit timer in Storm.qml, which ran from scene load. On a loaded machine Qt's image reader thread had not handed the provider a single request (or a cancel) when the timer fired. That produced "no requests at all" and "nothing was cancelled", and the two bad cases both looked like the ticket's reports. Storm.qml now starts its exit clock only when the provider has answered a canary request, and quits only after the grid has jumped at least 3 times since. The test keeps its clean-teardown checks per run and moves "quit in flight after cancellations" to a per-mode check with bounded retries. At the load where the old test failed 14 of 24 runs, the new one passed 200 of 200 runs.

acceptance criteria:
- [x] 0 failures in 100 runs alongside a loaded full-workspace run, measured.
  - Load: `cargo nextest run --workspace` looping in the background, 4 parallel copies of the test binary, 8 CPU burners. Nextest ran 64-128 s per round under it instead of ~35-60 s.
  - Old test and scene: 14 of 24 failed (58%). The messages were "storm 150 ms: no requests at all" (x8), "storm 300 ms: no requests at all" (x2), "storm 500 ms: no requests at all" (x1), "scroll 200 ms: no requests at all" (x1) and "scroll 200 ms: nothing was cancelled" (x2).
  - New test and scene: 100 of 100 passed, then another 100 of 100 passed.
  - Scene-level, 24 parallel `image-stress` plus 16 burners:
    - Old scene: `responses=0` in 55 of 96 runs (storm 150 ms), 77 of 96 (scroll 200 ms), 3 of 48 (storm 800 ms) and 13 of 48 (scroll 700 ms).
    - New scene (before the final change): 0 of 144 runs with `responses=0` or `cancels=0` (storm 150 ms and scroll 200 ms), 0 of 48 (storm 800 ms) and 0 of 96 (scroll 700 ms). The final scene differs only by dropping an unanswerable "flush" request.
- [x] The test still fails when cancellation at quit is broken (mutation).
  - A: removed `m_shared->cancels.fetch_add` in `ThumbResponse::cancel()` (`cpp/image_provider.cpp`). Fails with "no run quit with requests in flight after cancellations (storm 0, scroll 0) even after 6 extra attempts".
  - B: made `cancel()` also call `complete()`, which breaks ADR 0008 rule 3. Fails with "a second finished() was attempted left: 4 right: 0".
  - The cpp file was restored afterwards and its mtime touched.
- [x] ADR 0008 image-provider rules unchanged and still holding.
  - No C++, Rust provider or bridge code changed.
  - Every run still asserts exit 0, `STRESS_TEARDOWN_COMPLETE`, `duplicate_finishes == 0` and `finished <= responses`.
  - The canary and the (now removed) flush use the existing refuse path.
- [x] fmt, clippy -D warnings, nextest, deny pass (numbers below).

checks: fmt pass, clippy pass, tests 810 passed / 0 failed / 7 skipped (`cargo nextest run --workspace`, 59 s), deny pass (advisories, bans, licenses, sources ok). A second `cargo build -p tinwave-desktop` compiled nothing (0.13 s).

files changed (all under apps/tinwave-desktop/):
- tests/image_provider.rs
- tools/stress/Storm.qml
- tools/stress/main.rs (doc comment only)

signals added/changed: none
ui: none (Storm.qml is a dev-only stress fixture)
docs updated: none (the semantics are documented in the header of Storm.qml, in the `main.rs` doc and in the test comments)

What changed:
- Storm.qml:
  - An invisible "canary" Image asks for a 100000x100000 thumbnail. The provider refuses that at once on the reader thread, so its Image.Error proves the reader thread is running. Only then does the `--exit-ms` timer start.
  - The quit also needs `jumps >= 3` (the grid's jump timer counted since arming). Each jump drops its in-flight requests.
- tests/image_provider.rs:
  - The per-run `responses > 0` and `cancels > 0` asserts are gone. `responses > 0` would now hold trivially because of the canary.
  - New helper `quit_in_flight`: `in_flight_at_quit > 0 && cancels > 0`.
  - A `Covered` tracker records, per mode, the runs that satisfied it.
  - The extra attempts (still bounded at 6) now go to whatever is still missing: a storm run, a scroll run, or `tasks_run > 0`. This is because `tasks_run == 0` over all 7 runs showed up as a third failure ("the workers never ran a task") under extreme load.
- The test now takes about 20 s unloaded (it was 25 s before).

deviations from ticket:
- I touched `tools/stress/Storm.qml` and a doc comment in `tools/stress/main.rs`. These are outside the literal scope path `src/app/`. You covered the test file; the scene is the part of the same harness that holds the flaky timer, and the fix cannot be made in the test file alone. Please confirm this is acceptable.
- A "flush" request (a second refused request, to prove the reader thread had handled the cancels) turned out not to work. While a storm runs, the reader thread never gets to it: standalone and unloaded, a flush asked at 807 ms was still unanswered after 70 s. I removed it. The scene therefore cannot see whether the provider has already counted the cancels, so `cancels > 0` is a per-mode condition with retries and not guaranteed per run.

Load limits worth knowing:
- Under much heavier load (16 burners + 8 parallel copies + the looping workspace run, nextest ~10x slower), 15 of 100 test runs failed. 14 were "image-stress did not exit within 60s" (HANG_AFTER) and 1 was "no run quit with requests in flight after cancellations (storm 3, scroll 0) even after 6 extra attempts". In the 14 hangs STRESS_DONE had been printed but the 60 s limit passed before STRESS_TEARDOWN_COMPLETE.
- The old scene hits the same slowness: in a separate run of 7 sequential `image-stress` runs per driver under similar heavy load, one old-scene run took 69 s and none hung.
- I did not prove these 14 are slowness and not a hang. In that heavy-load run no `image-stress` ran longer than 94 s and none hung for 300 s in ~140 unloaded-by-test driver runs, but I did not look inside a stuck one.

proposals:
- P: Raise HANG_AFTER in tests/image_provider.rs from 60 s to ~180 s | why: healthy `image-stress` runs exceed 60 s at ~10x machine slowdown, so the hang detector gives false failures there; a real hang is infinite and is still caught | size: S | evidence: tests/image_provider.rs:20 (`HANG_AFTER = 60 s`); heavy-load driver logs, old scene max 69 s and new scene max 94 s, no run stuck at 300 s
- P: Investigate whether the 14 "STRESS_DONE printed but no STRESS_TEARDOWN_COMPLETE within 60 s" cases under extreme load are only slow `drop(shell)` or a real hang | why: ADR 0008 already records an old "2/300 hangs" class | size: M | evidence: scratchpad `heavy-n2.txt` (not in repo)
- P: A cheap observable for "cancels happened" in Storm.qml, e.g. a counter in the stress-only provider exposed to the scene | why: the scene could then gate on it and `cancels > 0` would hold per run | size: M | evidence: the unanswerable flush in this ticket

questions for lead:
- Is the `tools/stress/` edit acceptable under this ticket's scope?
- During my runs the machine was shared. A tinwave-db test (`signal_wipe::a_batch_that_outwaits_the_busy_timeout_is_stored_by_the_retry...`) failed once under my load run (122 s nextest round); it is unrelated to this ticket and passed in the final gate run. A stray `image-stress` from the T-0063 worktree was running when I finished; it is not mine.

All my processes (burners, loop scripts, test copies, `image-stress`) are stopped. Scratch scripts and logs are in the session scratchpad only. The worktree's `target/` can be deleted after merge.
