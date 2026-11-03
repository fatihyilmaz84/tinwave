REVIEW T-0066 (round 2)
verdict: CHANGES_REQUESTED
checks (run by reviewer): fmt pass, clippy pass (-D warnings), tests pass on the merged tree (main da69c7e + ca15d73, merge-tree d7136c0, no conflicts: 810 passed / 7 skipped), deny pass ("advisories ok, bans ok, licenses ok, sources ok"). In the branch worktree fmt, clippy and deny pass; nextest ran 803 with 802 passed and 1 failed: `tinwave-desktop::image_provider cancel_storm_with_requests_in_flight_then_quit_exits_cleanly` ("storm 150 ms: no requests at all"). That is the known T-0071 flake: it does not touch playback and it passed in the merged run. No ignored or real-device test was run and no audio device was opened. The ignored list still contains `tinwave-playback::device wasapi::exclusive_reopen_from_44_1_to_96_khz_is_timed_and_reported`.

acceptance criteria:
- [x] Ignored test compiles and is excluded from the default run — verified by: the default nextest run reports 7 skipped and the ignored list contains it.
  - Silence and system settings: re-read in round 2. The clips are still `vec![0.0; n]` WAVs. Nothing touches endpoint volume, mute, the default device or any Windows setting.
  - Failing loudly: if the start-up 48 kHz stream is not exclusive, `assert_eq!(first.mode, Exclusive, ...)` fails with the reason (device.rs:354).
  - Bounded: `play_pair` gives up after 5,000 rounds of 1 ms sleeps.
- [x] A refused rate costs one failed open + one open at the engine rate — verified by: `device::a_refused_rate_costs_one_failed_open_and_one_open_at_the_engine_rate` plus its companions, and the windows.rs unit tests.
  - Engine: matches `Err(error @ PlaybackError::ExclusiveRefused(_)) if request.exclusive_only` (engine.rs:968). Every other error is returned as before.
  - WasapiOutput: `open_error` maps only exclusive-only refusals (windows.rs:399-402). `REFUSED_PREFIX` must stay equal to `resolve_mode`'s text; `a_refusal_reaches_the_engine_as_the_refusal_error_and_a_failure_does_not` checks that against real `resolve_mode` output.
  - Leftovers: a grep of crates/ and apps/ finds no remaining use of `exclusive_refusal`, `exclusive_refusal_message` or `EXCLUSIVE_REFUSED`. The other "exclusive mode refused" strings are `StreamInfo::fallback` texts, not error matching. No exhaustive `match` on `PlaybackError` exists outside the crate.
  - T-0067 wrapper: `controller/setting.rs` `SharedOutput::open` (on main) forwards `request` and the result untouched.
- [x] settle() works in exclusive mode — verified by: `device::settle_works_in_exclusive_mode`. `settle` is unchanged since round 1 and its doc is fixed (tests/common/mod.rs:326-332).
- [x] fmt, clippy, nextest, deny pass — on the merged tree. In the worktree the only failure is the unrelated flake above.

Round-1 blocking finding (gap metric): fixed. On the fake, `the_boundary_gap_metric_sees_a_gap_of_a_known_size` asserts 40..=42 for a 40-tick open. On a real device, reading the code:
- **Start of the gap.** `service_reopen` opens only once `consumed >= reopen_total` and `idle_frames >= latency_frames` (engine.rs:873-879). The Pump reports latency as `buffer_frames + stream_latency_frames` (pump.rs:128). So the start of the 96 kHz open is the engine's estimate of when clip 1's last frame was heard. It is noticed at the next poll and counted in whole callbacks, so it lags the true end by up to one device period plus one poll. The gap is therefore slightly under-measured, never over-measured.
- **End of the gap.** `position` uses `played = consumed - latency` (engine.rs:1087-1091). The first poll with item 2 at frame > 0 is therefore the estimated heard time, already corrected for latency. It is not a consumption time, so it is not "up to one device latency late". Its error is up to one period plus one poll.
- **What it cannot see.** Both ends rest on the latency WASAPI reports. DAC or USB latency that WASAPI does not report is invisible; it shifts both ends about equally, so it roughly cancels.
- **Runbook.** It is honest: "good to a few ms", the poll caveat, and "the ear is the judge". It does not say the two ends are latency-model estimates (see non-blocking).

blocking findings:
- crates/tinwave-playback/tests/device.rs `the_boundary_gap_metric_sees_a_gap_of_a_known_size`, step closure (~line 740) with `play_pair(..., 20_000, ...)` — a failed reopen makes the test run for hours instead of failing.
  - **Mechanism:** after `OutputLost` there is no stream, so `out.buffered_frames()` is `None`, which becomes 0, and `reopen_pending()` is false. Every round then waits out the full inner loop of 3,000 × 100 µs, and `QueueEnded` never comes. That is at least 20,000 × 0.3 s ≈ 100 minutes, and longer with Windows sleep granularity. nextest never kills it: the only terminate-after override is for `storage_growth`.
  - **Reproduced** in a scratch copy of the merged tree. I made `SlowOpen::open` return `PlaybackError::Output("device gone")` at 96 kHz; the engine reports `OutputLost`, as it legitimately does for a device that vanishes. The test was still running after 120 s ("has been running for over 60 seconds") and I killed it.
  - **Failure scenario:** any future regression in the reopen path makes CI or a builder's run hang for hours, which is the kind of hang the builder already hit during a mutation.
  - **Fix:** in `play_pair`, panic as soon as the sink shows `OutputLost` or `TrackFailed`. That also makes the device test fail at once instead of after 5 s. In the step closure, also stop waiting when `!out.is_open()`. Optionally lower the inner cap.

design check: n/a

non-blocking findings:
- PO-RUNBOOK.md section 7, "The number that matters for the gap": suggested addition — "both ends are Tinwave's estimates from the buffer size and the latency Windows reports, accurate to about one device period plus the poll; latency inside the DAC itself is not seen".
- crates/tinwave-playback/tests/device.rs, the `GAP between the clips` println (~line 408): "(the 44.1 kHz stream had played out ...)". After a 44.1 kHz refusal that stream is the 48 kHz one.
- crates/tinwave-playback/src/output/mod.rs:56-57 — the rewrapped doc line runs past 100 columns. rustfmt does not wrap comments, so fmt passes.
- Runbook section 7 has the rest of the requested changes: the restore-your-default-output wording, the polls line, the 44.1 kHz refusal case, and the new metric. It is still a pure append versus main (86/0); sections 1-6 are untouched.
- The windows.rs module doc (lines 19-24) and the `settle` doc are fixed.

proposals:
- P: nextest default backstop for playback test binaries (a slow-timeout terminate-after, as for storage_growth) | why: several playback tests use spin-and-sleep loops whose worst case is minutes to hours under a regression; a backstop turns a hang into a failure | size: S

notes for lead:
- The merge with current main (T-0064 and the checkpoint fix) is clean, and the merged tree passes all four gates.
- T-0067's `SharedOutput` needs nothing for this change, since no trait method was added.
- The `exclusive_reopen` hang the builder hit was in a file this branch does not touch. I did not look for long-loop risks there; the proposal above would cover that too.
- I deleted every scratch file I created (the merged archive copy, its target dir, the logs and the mutation output). No process of mine is left running; the mutated test process was killed and confirmed gone.
