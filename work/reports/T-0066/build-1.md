HANDOFF T-0066 (round 1)
status: done
branch: t/T-0066-exclusive-reopen-device-check   commit: fde6b88
summary: Added `OpenRequest::exclusive_only` (the engine sets it for a file rate that is not its own in exclusive mode). `WasapiOutput` and `ManualOutput` then fail with a typed refusal instead of falling back to shared, so a refused rate costs one failed open plus one open at the engine rate. Added an `#[ignore]`d, silent-only 44.1 to 96 kHz exclusive-reopen device test that times every open. Added four always-run no-device tests and made `Harness::settle` poll while a reopen is pending. Added PO-RUNBOOK section 7. No device or ignored test was run; I only listed the ignored tests.

acceptance criteria:
- [x] Ignored test compiles and is excluded from the default run. Proof: `cargo nextest run --workspace` gives "777 tests run: 777 passed, 7 skipped", and `tinwave-playback::device` shows 5 skipped. `cargo nextest list -p tinwave-playback --test device --run-ignored ignored-only` (list only, runs nothing) shows `wasapi::exclusive_reopen_from_44_1_to_96_khz_is_timed_and_reported` in the ignored set. The clips are all-zero WAVs. The test sets no system volume, device or Windows setting, and it never runs from this ticket.
- [x] A refused rate costs one failed open + one open at the engine rate. Proof: `device::a_refused_rate_costs_one_failed_open_and_one_open_at_the_engine_rate`. `opens() == [48000, 96000, 48000]`, the new `ManualHandle::refused_opens() == 1`, and opens minus refused is 2 streams. The refusal is reported once with "96000 Hz refused ... resampling".
  - Companions: `an_accepted_rate_is_one_open_and_the_engine_rate_is_never_exclusive_only`, `an_output_that_ignores_exclusive_only_still_gets_the_refusal_handled` (old-style output keeps working, costing the wasted open), and `a_device_failure_at_a_new_rate_is_not_taken_for_a_refusal`.
- [x] settle() works in exclusive mode. Proof: `device::settle_works_in_exclusive_mode`.
- [x] fmt, clippy, nextest, deny pass. fmt pass, clippy `-D warnings` pass, nextest workspace 777 passed / 0 failed / 7 skipped, deny "advisories ok, bans ok, licenses ok, sources ok".

Mutations tried (each reverted; the tree is clean):
- Engine never sets the flag: the refused-open-count test and the ignoring-output test fail.
- Engine stops checking for a shared stream: the ignoring-output test fails.
- Any `Err` treated as a refusal: `a_device_failure_at_a_new_rate...` fails. Without the typed refusal, 3 existing tests failed (`engine::tests::a_device_that_cannot_open_is_reported_lost_and_not_asked_again`, `engine::tests::a_lost_output_is_not_polled_quickly`, `boundary::a_failed_first_choice_does_not_leak_the_next_tracks_position`).
- `settle` stops polling: `settle_works_in_exclusive_mode` and the three open-count tests fail via the 30,000-round panic, about 15 s.
- `require_exclusive` and `open_error` (windows.rs unit tests) mutated: both caught.

checks: fmt pass, clippy pass, tests 777 passed / 0 failed (7 skipped = the ignored ones), deny pass
- Under heavy machine load (CPU at 100% from other agents) one run had `rate::a_drag_costs_a_few_flushes...` and `rate::no_click_at_a_rate_step...` fail (one flush for 48 steps over 492 ms). These are wall-clock tests that pass in about 1.5 s when the machine is quiet. They do not use anything I changed: `settle` in shared mode behaves as before, because `reopen_pending()` is false there.
- The final full workspace run was clean.

files changed:
- crates/tinwave-playback/src/output/mod.rs
- crates/tinwave-playback/src/output/windows.rs
- crates/tinwave-playback/src/engine.rs
- crates/tinwave-playback/tests/device.rs
- crates/tinwave-playback/tests/common/mod.rs
- spikes/T-0003-audio-backend/PO-RUNBOOK.md

signals added/changed: none
ui: none
docs updated: PO-RUNBOOK section 7 only. Sections 1-6 are untouched; it is a pure append of 76 lines.

What the code does:
- **`OpenRequest::exclusive_only`:** `exclusive && rate != engine rate`, always false for the start-up open. `Engine::open_stream` treats an exclusive-only open as refused in two cases: an `exclusive_refusal` error, or (for an output that ignores the flag) a non-exclusive stream, which it closes. It remembers the rate and retries at the engine rate, as in T-0057.
- **`ManualOutput`:** honours the flag and counts refused opens in `refused_opens()`, a small addition to the handle.
- **`WasapiOutput`:** `open_device` fails with the "exclusive mode refused: ..." reason before building a shared client. `open_error` turns that into `exclusive_refusal`.
- **Ignored device test:** `exclusive_reopen_from_44_1_to_96_khz_is_timed_and_reported`.
  - It wraps the real `WasapiOutput` in a `TimedOutput` that records each open's rate, flag, duration and result.
  - It prints every open (rate, `(exclusive only)`, ms, mode/format/period/buffer or `REFUSED ...`), load to first reopen, reopen requested to new stream announced, and last frame of clip 1 to first frame of clip 2 (polled every ~1 ms).
  - It asserts only what holds on any endpoint: the flag is set exactly for rates other than 48 kHz, an exclusive-only success is exclusive, a refusal is followed by a 48 kHz non-exclusive-only open and is reported, the 96 kHz clip is heard, and there is no `TrackFailed` or `OutputLost`.
  - If the endpoint does not open 48 kHz exclusively at start-up, it fails with that reason rather than passing silently.
- **Runbook section 7:**
  - It gives the exact `cargo test -p tinwave-playback --test device -- --ignored --nocapture --test-threads=1 exclusive_reopen_from_44_1_to_96_khz_is_timed_and_reported` command (no Qt environment needed; I built it without Qt).
  - It describes the printed block, how to read the three outcomes (96 kHz accepted, 96 kHz refused, 48 kHz not exclusive), what to watch on the DAC, and what to report back.
  - It tells the PO to lower the amp level on the device itself and to set Windows default-output and exclusive-control prerequisites by hand; the test changes none of that.

deviations from ticket:
1. **A typed refusal helper in `output/mod.rs`:** `pub fn exclusive_refusal(reason)`, plus `pub(crate)` `EXCLUSIVE_REFUSED` and `exclusive_refusal_message`, beyond the "flag only" wording. Without it, a generic open failure at a new rate (device gone) was treated as a refusal. That retried at the engine rate and remembered the rate as refused forever, and the 3 existing tests above failed. The engine now treats only this error as a refusal.
2. **`Harness::settle` polls only while `engine.reopen_pending()` is true,** not on every round as `settle_polling` does. Polling unconditionally broke `boundary::a_failed_first_choice_does_not_leak_the_next_tracks_position`, which relies on no poll having announced item 2 after `settle`. The two behave the same in shared mode, and `settle_polling` is kept as before. `boundary.rs` is out of scope, so I did not change that test.
3. **`Harness::exclusive_through(device_rates, wrap)`,** a small helper in `tests/common/mod.rs`, builds the harness around a wrapped output.
4. **The ticket's "listen for the gap on a real album" cannot be done with silent clips,** and no audible app path exists yet. The runbook says so and has the PO watch the DAC's rate indicator and listen for relay clicks at the 44.1 to 96 kHz switch and at stream close, plus the printed gap numbers. Nothing audible is run.

proposals:
- P: Re-export `exclusive_refusal` from `lib.rs` | why: it is `pub` in `output` but `lib.rs` (out of my scope) re-exports a fixed list, so out-of-crate `AudioOutput` implementors (desktop, tests) cannot return the typed refusal | size: S | evidence: crates/tinwave-playback/src/lib.rs:61 and output/mod.rs `exclusive_refusal`
- P: Fix PO-RUNBOOK section 6's command | why: `\target\release\t0003-...` was written with unescaped backslashes, so the file now contains a TAB and a CR (the line renders as ".<TAB>arget<CR>elease<TAB>0003-audio-spike.exe sysvol 15", and `file` reports mixed CR/LF). I left it untouched because the ticket says to keep existing sections intact | size: S | evidence: spikes/T-0003-audio-backend/PO-RUNBOOK.md, section 6 code block
- P: Audible, PO-supervised real-album variant of the device test (path from an env var) | why: the only way to hear the actual inter-track gap. It is not silent, so it needs explicit PO approval and a volume procedure | size: S | evidence: runbook section 7, last paragraph
- P: Optionally add `sample_rate` to `StreamInfo` | why: today only a wrapper output can report the rate a real stream opened at (my `TimedOutput` does it in the test) | size: S | evidence: output/info.rs

questions for lead:
- Is the `exclusive_refusal` helper in `output/mod.rs` acceptable as part of the "flag" change (deviation 1), and do you want the `lib.rs` re-export as a follow-up?
- Should `boundary.rs` (or other tests relying on settle not polling) be adjusted in a later ticket so `settle` can poll unconditionally, as the ticket literally said? The conditional poll covers the stated goal (no spin in exclusive mode).
- This is for the T-0057 ADR "Not verified" note: the runbook and test exist now, but the measurement is the PO's to run. I did not edit docs/adr (out of scope).
