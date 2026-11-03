REVIEW T-0071 (round 1)
verdict: APPROVE
checks (run by reviewer): fmt pass (exit 0); clippy -D warnings pass (exit 0); build `cargo build --workspace` pass (exit 0); tests pass, `cargo nextest run --workspace` "810 tests run: 810 passed, 7 skipped" on a quiet machine (59.8 s), and passed twice more under load (59.9 s, and 159.5 s with 4 slow); deny pass ("advisories ok, bans ok, licenses ok, sources ok")

acceptance criteria:
- [x] 0 failures in 100 runs alongside a loaded full-workspace run, measured. Verified by:
  - The builder's 200/200, which I corroborated with my own runs.
  - 24 runs as 4 parallel copies of the test binary. During the first round a workspace nextest was also compiling.
  - 24 runs as 3 parallel copies while two full `cargo nextest run --workspace` rounds ran back to back. The second round slowed to 159 s and individual test runs took up to 70 s.
  - 3 more runs inside the nextest rounds.
  - Result: 51/51 passed, 0 failures. Logs are in C:\Users\FSYY\AppData\Local\Temp\claude\D--dev-Tinwave\53bf9027-c019-4dd4-82f1-5a72c6705b0c\scratchpad\flake\
- [x] The test still fails when cancellation at quit is broken (mutation). Verified in a `git archive` scratch copy with its own CARGO_TARGET_DIR; tinwave-desktop recompiled each time; the copy is deleted.
  - A: dropped `cancels.fetch_add` in `ThumbResponse::cancel()`. FAILED: "no run quit with requests in flight after cancellations (storm 0, scroll 0) even after 6 extra attempts" (25 s).
  - B: `cancel()` also calls `complete()`. FAILED on the first run: "a second finished() was attempted, left: 2 right: 0".
  - C (my extra, for the "refuses everything" question): the refuse branch forced to `if (true)`. FAILED: "(storm 1, scroll 0) even after 6 extra attempts". The `tasks_run > 0` assert would also have caught it.
  - D (my extra): the refused path never completes, so the canary is never answered. FAILED after 60 s: "image-stress did not exit within 60s (hang at exit)".
- [x] ADR 0008 image-provider rules still hold. Verified by:
  - `git diff main...HEAD --stat` touches only `tests/image_provider.rs`, `tools/stress/Storm.qml` and a doc comment in `tools/stress/main.rs`. There is no C++, Rust provider or bridge change.
  - The canary uses the existing refuse path (`image_provider.cpp:136-141`). A direct run shows "Storm.qml:122:5: QML Image: thumbnail size 100000x100000 is above the limit of 1024 px per edge".
  - Every run still asserts exit 0, STRESS_TEARDOWN_COMPLETE, `duplicate_finishes == 0` (`image_provider.rs:51-64`) and `finished <= responses` (`image_provider.rs:163-166`).

Focus answers:
1. The test did not get weaker in what it proves.
   - Before, every run needed `cancels > 0`, and each mode needed at least one run with `in_flight_at_quit > 0`. So the qualifying run already had both.
   - Now each mode needs at least one run where both hold (`quit_in_flight`, `image_provider.rs:176-178`), and that run has passed `expect_clean_exit`. The proof is equivalent.
   - The only loss: the per-run `cancels > 0` no longer fails a run quickly. A broken cancel counter now fails after the 7 + 6 bounded runs, not on the first one.
   - `responses > 0` would be trivially true now because of the canary, so dropping it is correct.
   - Teardown is still checked on every run, extras included.
2. The canary gate.
   - It cannot hide a defect. Refuse-all fails (mutation C). A provider that never answers fails (D, as a 60 s hang). Allow-all would still end in Image.Error, because the 100000² QImage is null, so the run arms; the Sizes tests cover the edge limit.
   - It does not change production: the provider code is untouched.
3. `jumps >= 3` cannot stop a run from quitting. The jump timer (`Storm.qml:46-57`) runs for the scene's whole life, and `maybeQuit()` is called from both the jump timer and the quit timer, so either order quits. The one thing that can stop the quit is the canary never being answered (see the first non-blocking finding).
4. Flake rate: 0 failures out of 51 runs (detail above).
5. ADR 0008: holds (see the third criterion).

blocking findings:
- none

design check: n/a (Storm.qml is a dev-only test fixture, not under qml/)

non-blocking findings:
- `apps/tinwave-desktop/tools/stress/Storm.qml:122-135` with `tests/image_provider.rs:126-131`: the quit now depends on the reader thread reaching the canary.
  - The builder showed that a request made during a running storm can wait more than 70 s. The canary only works because it is queued before the storm starts.
  - If the canary ever starves, the run never quits. After 60 s it is reported as "hang at exit", with no STRESS_DONE line, which is the wrong diagnosis. My mutation D shows exactly this message for a scene that never quit.
  - Today it fails loudly, not silently, and I saw no such case in 51 loaded runs.
- `apps/tinwave-desktop/tools/stress/run-stress.ps1:2,6-7`: the header still says "random quit time", and the example `-ExitMin 50` still implies a quit 50 ms after load.
  - Both now mean "after the canary is answered, plus at least 3 jumps". The Release stress target (ADR 0008 rule 6) therefore no longer covers a quit before the provider has answered anything.
  - This is a doc and coverage nuance, not a regression of the rule 5 cases: late requests after the quiesce still happen.

proposals:
- P: Tell "never quit" apart from "hang at exit" in `run_stress` (no STRESS_DONE in stdout means the scene never called Qt.quit) | why: a starved canary or a dead provider currently reads as a teardown hang | size: S
- P: Update the `run-stress.ps1` header and `-ExitMin` help to say the quit time now counts from the provider's first answer | why: ADR 0008 rule 6 describes the stress as "random exit 200–900 ms" | size: S
- P: (builder's) raise HANG_AFTER to about 180 s | why: under heavy load, healthy runs can exceed 60 s; my loaded runs took up to 70 s per test (across several image-stress runs), but no single image-stress run came near the limit | size: S

notes for lead:
- Scope: `tools/stress/` was accepted by your decision. The diff has no `work/` edits and no TODOs.
- Running `cargo test -p tinwave-desktop --no-run` and then `cargo nextest run --workspace` in the worktree relinks tinwave-desktop each time, because the two commands resolve features differently. My first loaded nextest therefore failed to replace a running `image-stress.exe` (os error 5). That was my test setup, not a defect, and later rounds passed.
- Nothing is left running. The scratch copy and its target dir are deleted. Flake logs and scripts remain only in C:\Users\FSYY\AppData\Local\Temp\claude\D--dev-Tinwave\53bf9027-c019-4dd4-82f1-5a72c6705b0c\scratchpad\
- Free disk on D: was about 5.6 GB during the review.
