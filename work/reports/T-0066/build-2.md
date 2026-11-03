HANDOFF T-0066 (round 2)
status: done
branch: t/T-0066-exclusive-reopen-device-check   commit: ca15d73 (main was merged first, merge commit 624664c)
summary: The blocking gap metric is fixed. The gap is now "start of the open for the 96 kHz clip -> first poll that hears it". `TimedOutput` records each open's start on a tick clock, which is nanoseconds on a real device and loop rounds on `ManualOutput`. A new non-ignored test runs the same loop and metric against a fake device whose open takes exactly 40 ticks. The string-prefix refusal is replaced by a real `PlaybackError::ExclusiveRefused(String)`, and the `exclusive_refusal` helper and lib.rs re-export are gone. The runbook and the two stale doc comments are fixed. No ignored or real-device test was run and no audio device was opened.

acceptance criteria:
- [x] Ignored test compiles and is excluded from the default run. Proof: nextest workspace "803 tests run: 803 passed, 7 skipped". `device` runs 7 tests with 5 skipped, and the ignored list still contains `wasapi::exclusive_reopen_from_44_1_to_96_khz_is_timed_and_reported`. The clips are all-zero WAVs; nothing sets endpoint volume, device or Windows settings.
- [x] A refused rate costs one failed open + one open at the engine rate: `device::a_refused_rate_costs_one_failed_open_and_one_open_at_the_engine_rate`, now matching `ExclusiveRefused`. Companions: `an_accepted_rate_is_one_open...`, `an_output_that_ignores_exclusive_only...`, `a_device_failure_at_a_new_rate_is_not_taken_for_a_refusal`.
- [x] settle() works in exclusive mode: `device::settle_works_in_exclusive_mode`.
- [x] fmt, clippy, nextest, deny pass.

round-2 items:
- **BLOCKING gap metric.**
  - `OpenLog.started` is the tick when the `open` call begins.
  - `boundary_gap(log, 96000, run)` = first poll where item 2 has frame > 0, minus the start of the open that asked for 96 kHz.
  - The ignored test prints `GAP between the clips: open #N start -> 96 kHz clip heard: X ms`. It also keeps `open #N took`, "load -> first reopen announced" and "load -> 44.1 kHz clip heard".
  - `last_a` and "reopen requested -> new stream announced" are removed.
  - New non-ignored test `device::the_boundary_gap_metric_sees_a_gap_of_a_known_size`:
    - The test drives the same `play_pair` loop, `TimedOutput` and `boundary_gap` as the device test.
    - `SlowOpen` wraps `ManualOutput` and makes each open take 40 ticks, one tick per 48-frame callback. The device latency is 0.
    - It asserts `took == 40` and a reported gap in 40..=42; it observes 41.
    - Mutation: with the old metric (first_b minus the last poll that saw item 1) the test fails with "a gap of 40 ticks was reported as 1; opens [(48000, 0), (44100, 40), (96000, 181)]". The file was restored.
- **`PlaybackError::ExclusiveRefused(String)`**, Display "exclusive mode refused: {0}".
  - `ManualOutput` returns it.
  - `WasapiOutput::open_error` maps the render thread's text (prefix `REFUSED_PREFIX`, private to windows.rs) to it only when the request was exclusive-only. Any other failure stays `PlaybackError::Output("WASAPI: ...")`.
  - The engine matches `Err(error @ ExclusiveRefused(_)) if request.exclusive_only`. Everything else still returns the error as before.
  - `exclusive_refusal`, `exclusive_refusal_message` and `EXCLUSIVE_REFUSED` are removed.
  - `controller/setting.rs` `SharedOutput::open` forwards `request` and returns the inner result untouched, so it needs no change (checked after the merge).
  - Mutation: with the engine arm disabled (`if false && ...`), `device::a_refused_rate_costs_one_failed_open_and_one_open_at_the_engine_rate` fails. An earlier run of the same mutation over `exclusive_reopen` hung, so I stopped those two test processes myself and the file was restored. The windows.rs unit test `a_refusal_reaches_the_engine_as_the_refusal_error_and_a_failure_does_not` now matches the variant. Its flag-ignoring mutation was caught in round 1.
- **Runbook section 7:**
  - Step 1 now says to note the current default output and restore it afterwards.
  - The sample block includes the "(this loop polls every ~1 ms ...)" line and the `GAP` line.
  - "How to read it" describes the new metric and no longer assumes only 96 kHz can be refused (it covers a 44.1 kHz refusal, where `open #2` is the 48 kHz retry).
  - It mentions the known-gap test.
  - The file is still a pure append versus main (86 added, 0 removed).
- **Stale docs:**
  - `output/windows.rs` module doc now says a refused rate is an `ExclusiveRefused` error for exclusive-only opens, with shared fallback only at the engine's own rate.
  - `tests/common/mod.rs` no longer says "settle_polling used to"; it states that `settle_polling` polls every round and `settle` only while a reopen is pending.
  - The `OpenRequest` doc is updated for the new variant.

checks: fmt pass, clippy pass (`-D warnings`, workspace, all targets), tests 803 passed / 0 failed / 7 skipped, deny pass ("advisories ok, bans ok, licenses ok, sources ok")

files changed:
- crates/tinwave-playback/src/error.rs (new variant; authorised by your round-2 message, outside the original scope)
- crates/tinwave-playback/src/engine.rs
- crates/tinwave-playback/src/output/mod.rs
- crates/tinwave-playback/src/output/windows.rs
- crates/tinwave-playback/tests/device.rs
- crates/tinwave-playback/tests/common/mod.rs
- spikes/T-0003-audio-backend/PO-RUNBOOK.md

signals added/changed: none
ui: none
docs updated: PO-RUNBOOK section 7
deviations from ticket: error.rs touched at your request (the new variant). Items 1 and 2 from round 1 remain: `Harness::settle` polls only while `reopen_pending()` is true, and the `Harness::exclusive_through` helper exists.

proposals:
- P: none new. The `exclusive_refusal` re-export proposal from round 1 is withdrawn because the variant replaces it.

questions for lead: none
Nothing of mine is still running (checked by command line for this worktree).
