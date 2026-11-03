REVIEW T-0066 (round 1)
verdict: CHANGES_REQUESTED
checks (run by reviewer): fmt pass, clippy pass (-D warnings), tests pass (branch worktree: nextest 777 passed / 7 skipped; merged tree main 8e26816 + fde6b88: 799 passed / 7 skipped), deny pass ("advisories ok, bans ok, licenses ok, sources ok"). Both trees: all four gates exit 0. No ignored or real-device test was run and no audio device was opened. I only ran `nextest list --run-ignored ignored-only`, which lists 7 tests including `tinwave-playback::device wasapi::exclusive_reopen_from_44_1_to_96_khz_is_timed_and_reported`.

acceptance criteria:
- [x] Ignored test compiles and is excluded from the default run — verified by: the default nextest run reports 7 skipped and the ignored-only list contains the new test. Read-only checks of the test (crates/tinwave-playback/tests/device.rs ~259-421):
  - Both clips are `vec![0.0; n]` WAVs. Dither leaves exact zeros alone (convert.rs policy plus its silence test).
  - `set_volume(0.0)` is the software gain. `control.set_exclusive(true)` is the app's own WasapiControl. Nothing touches endpoint volume, mute, the default device or any Windows setting.
  - If the start-up 48 kHz stream is not exclusive, the test fails at once with the device name and the reason (`assert_eq!(first.mode, Exclusive, ...)`).
- [x] A refused rate costs one failed open + one open at the engine rate — verified by: `device::a_refused_rate_costs_one_failed_open_and_one_open_at_the_engine_rate` (opens [48k, 96k, 48k], refused_opens == 1, one fallback note). Companions:
  - `an_output_that_ignores_exclusive_only_still_gets_the_refusal_handled`
  - `a_device_failure_at_a_new_rate_is_not_taken_for_a_refusal` (a generic Err stays OutputLost with no retry)
  - `an_accepted_rate_is_one_open...`
  - the four windows.rs unit tests for `require_exclusive` / `open_error`
  Code: `require_exclusive` runs before the shared client is rebuilt (windows.rs:324). The flag is `exclusive && rate != self.sample_rate` (engine.rs:949) and is false at start-up (engine.rs:222). The engine treats only an `exclusive_refusal_message` error, or a non-exclusive stream, as a refusal (engine.rs:952-973). Refusals go into `refused`, which `switch_device` clears (engine.rs:637).
- [x] settle() works in exclusive mode — verified by: `device::settle_works_in_exclusive_mode`. `Harness::settle` (tests/common/mod.rs:332-344) polls only when `reopen_pending()`, and every round still sleeps 100 µs, so it does not spin. In shared mode `reopen_pending` is always false because the decoder only parks when `sh.follow` is set (decoder.rs:584), so shared-mode behaviour is unchanged.
- [x] fmt, clippy, nextest, deny pass — see checks (run on both the branch and the merged tree).

blocking findings:
- crates/tinwave-playback/tests/device.rs ~297-315 / ~352-356 and spikes/T-0003-audio-backend/PO-RUNBOOK.md section 7 ("The number that matters for the gap") — the headline gap measurement cannot see the gap.
  - **Defect:** `last_a` is updated on every poll that sees item 1 with frame > 0. `Engine::position` deliberately stays on item 1 through the drain and the reopen (T-0057, `exclusive_reopen::the_position_stays_on_the_old_track_until_the_new_stream_is_heard`). So `last_a` keeps moving until the poll just before `first_b`, and "last frame of the first clip -> first frame of the second" always comes out as about one poll (~1-2 ms), whatever the real gap.
  - **Reproduced** on ManualOutput in a scratch copy of the merged tree (the device test's loop, one 48-frame callback per iteration, device latency 2,400 frames). The real gap was 50 callbacks with no audio (iterations 370-419); the test's metric gave 2 iterations. Item 1's frame stayed frozen at 16588 during the gap and then advanced again after the new stream opened, so "last time the frame changed" does not work either.
  - **Second metric is also wrong:** "reopen requested -> new stream announced" starts when the decoder parks. With a 1 s ring and a 0.4 s clip, that can be long before clip 1 has played out (in the repro it started 200 callbacks before the gap). So it measures the rest of clip 1's playback, not the gap.
  - **Failure scenario:** the PO runs section 7 and reports a gap of about 1 ms, which the runbook tells them to compare with the spike's 42 ms. The T-0057 ADR "Not verified" note would then record a false measurement. Measuring this gap is the reason the test exists.
  - **Fix:**
    - In `TimedOutput`, record the `Instant` each open starts. The engine only opens once the old buffer has drained (idle_frames >= latency), so that is when clip 1 stops sounding.
    - Report the gap as "open #N start -> first poll where item 2 has frame > 0". That covers the open, the first block, the new stream's latency and the poll.
    - Keep `open #N took`, drop or relabel the `last_a` number and the requested -> announced number, and update section 7 to match.

design check: n/a

non-blocking findings:
- spikes/T-0003-audio-backend/PO-RUNBOOK.md section 7 "Before you start" step 1 asks the PO to make the DAC the Windows default output, and never says to switch it back. Suggested wording: "if it is not already; note the current default and restore it afterwards". It is a manual, reversible step, but you asked for nothing that changes the PO's settings. Everything else in the section is a read or a physical device knob. Section 7 is a pure append; sections 1-6 are untouched (numstat 76/0).
- PO-RUNBOOK section 7:
  - The sample block leaves out the "(this loop polls every ~1 ms ...)" line the test prints.
  - The open numbering in "How to read it" assumes only 96 kHz can be refused. If 44.1 kHz is refused, `open #2` is the 48 kHz retry.
- crates/tinwave-playback/src/output/windows.rs:19-22 — the module doc still says a refused rate "comes back as a shared stream at that rate". In exclusive mode that no longer happens for any non-engine rate.
- crates/tinwave-playback/tests/common/mod.rs:329-331 — "like `settle_polling` used to": `settle_polling` still polls on every round. The wording suggests it changed.
- crates/tinwave-playback/src/output/mod.rs:76-80 — the "typed" refusal is matched by a string prefix on `PlaybackError::Output`. This is accepted by the lead decision. Any output error text that happens to start with "exclusive mode refused" on an exclusive-only open would count as a refusal.

proposals:
- P: `PlaybackError::ExclusiveRefused(String)` variant instead of the message-prefix match | why: a real type cannot be forged by accident and works for out-of-crate outputs without re-exporting a helper | size: S
- P: Position during an exclusive drain freezes and then resumes after the reopen | why: in the repro, item 1's frame stopped at 16588 for the whole drain while the device was still playing its buffered tail, then advanced again once the new stream ran. The heard-position model ignores idle frames during the drain (seen on ManualOutput, not on a real device) | size: S

notes for lead:
- The merge with current main is clean: `git merge-tree --write-tree main t/T-0066...` gives tree d68c31a with no conflicts. The merged tree passes all four gates. main's T-0062/T-0065 controller changes touch neither OpenRequest nor engine.rs. The only out-of-module AudioOutput on main, `InfoOutput` in tests/controller_support, passes `&OpenRequest` straight through.
- T-0067: this branch adds no AudioOutput trait method, only the field `OpenRequest::exclusive_only` and the typed error. T-0067's `SharedOutput` (controller/setting.rs, f97fd0f) already forwards `request` unchanged and returns the inner error untouched, so it needs no change. It must keep passing the same `&OpenRequest` (not rebuild it) and must not re-wrap `open` errors, or the refusal stops being recognised and degrades to OutputLost.
- Under machine load, `a_refused_rate_costs...` took 8.2 s in the merged run (0.5 s when quiet). It passed, but it is wall-clock sensitive like the other settle-based tests.
- The scratch copy (`git archive` of the merged tree, with its own target dir) and every file I created are deleted. No cargo or test process of mine is left running; the cargo/nextest `cancel_storm` stress run still active belongs to another agent.
