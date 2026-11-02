REVIEW T-0024 (round 2)
verdict: APPROVE
checks (run by reviewer): fmt pass (exit 0), clippy pass (`--workspace --all-targets --locked -- -D warnings`, exit 0), tests pass (`cargo nextest run --workspace --locked`: 454 passed, 0 failed, 6 skipped; the 6 skipped are ignored device/keychain tests, which I did not run), deny pass (advisories, bans, licenses and sources all ok)

flakiness: I ran the controller_thread, controller_signals and controller_sessions tests 8 times in a row through nextest (48/48 each time). I also ran the controller_thread and controller_signals binaries 6 copies at a time, 3 rounds (36 runs). Every run passed.

acceptance criteria:
- [x] One emission test per mapped kind. Unchanged from round 1 and still green. The 4 new tests in tests/controller_signals.rs also check payload and session for the deferred RepeatModeChanged and ExclusiveModeToggled.
- [x] Failed gives neither Completed nor Skipped; seek-to-end gives SeekForward + Completed; a seek never merges across a track change. Same tests as round 1, all green.
- [x] Session start and end at 29:59 vs 30:00. tests/controller_sessions.rs is green 8 times out of 8.
- [x] A disabled category constructs nothing. Unchanged; `report_repeat` and `report_mode` still build payloads inside the `send` closure (mapper.rs:332-350).
- [x] Critical events are never dropped by the sink. The controller_sink tests are green.
- [x] Zero callback allocations, pairing property green, no wall-clock reads. rt_alloc, output_rt_alloc, controller_rt_alloc and pairing are green. The new code reads time only from the `at` parameter or `self.clock.now()` (mod.rs:351).
- [x] fmt, clippy, tests and deny pass, and the default run opens no device. The device tests stay `#[ignore]`d. The new stuck-decoder test uses ManualOutput and GatedOpener.

Round-1 blocking findings:
1. Exit never waits for the decoder: fixed.
   - `Controller::close` (mod.rs:341) runs `wind_down` (stop, drain, `mapper.finish`, so SessionEnded is still emitted), then drops the Engine. The joining `shutdown` (mod.rs:333) is now only for tests.
   - The ControllerHandle thread ends with `controller.close()` (runtime.rs:85). Both `drop` and `shutdown` go through `stop_thread`, which joins only the controller thread.
   - Nothing on the close path waits on the decoder. `Engine::stop` sends with `try_send` (engine.rs:608). `Engine::drop` (engine.rs:704-708) sets `stream = None`, which closes the output stream and device, then sets the shutdown flag and does a `try_send` wake-up.
   - The detached decoder thread is safe Rust: crate-level `#![deny(unsafe_code)]`, and the only `unsafe` is in output/windows/os.rs. It holds only Arcs and an rtrb producer, and it exits on `sh.shutdown` (decoder.rs:121). So there is no undefined behaviour and the device does not stay open.
   - Test `quitting_does_not_wait_for_a_decoder_stuck_in_a_read` (tests/controller_thread.rs:296):
     - The first read blocks (`allow: 0`), and the test waits until `gate.blocked` is set inside the read.
     - It then drops the handle on a helper thread and requires it to return within 5 s, opening the gate only after that timeout.
     - With `shutdown()` the controller thread would join the decoder parked in `Gate::wait`, so `recv_timeout` would fail and the assert would fire. The test really detects a joining teardown. I reasoned this through rather than editing the worktree, which is read-only.
     - It also checks [SessionStarted, PlayStarted, SessionEnded] on the way out.
2. Preference changes made outside a session: fixed.
   - `set_repeat` (mapper.rs:199) and the OutputReopened path (mapper.rs:572-575) only record the change when there is no session.
   - `ensure_session` then calls `report_preferences` right after SessionStarted (mapper.rs:317-330). A value changed back before any session is not reported, because it is compared with `reported_*`.
   - Event order is SessionStarted, then RepeatModeChanged and/or ExclusiveModeToggled, then PlayStarted. In the stale-play path it is SessionStarted, pref change(s), Skipped/Completed, PlayStarted. The catalog (§4 row 58, §6 row 98, §8 row 131) states no ordering rule, and every event lands inside the session it belongs to, so this is consistent.
   - Tests: `a_repeat_mode_set_with_nothing_loaded_is_reported_by_the_next_session`, `a_repeat_mode_changed_back_before_any_session_is_not_reported`, `a_repeat_mode_set_after_a_session_ended_is_reported_by_the_next_one`, `an_exclusive_mode_change_outside_a_session_is_reported_by_the_next_session`.

Small items, all done:
- The load-race log is now `debug` with an explanation (mapper.rs:370-378).
- Seek with an expired session is documented in the "Gestures need a session" note (mod.rs:70-72).
- `switch_device` blocking is documented in mod.rs:16-21, in the `Controller::switch_device` doc and in the `ControllerHandle` doc (runtime.rs:43-47).
- `TimezoneSource` must not block (types.rs:104-107).

All changes are within scope: controller/{mod,mapper,runtime,types}.rs and two test files.

blocking findings:
- none

design check: n/a (no UI)

non-blocking findings:
- crates/tinwave-playback/src/controller/mapper.rs:199 + types.rs:129-152 — `ControllerConfig` has no initial repeat mode; the mapper always starts with `Off`. If the E-01 shell later saves the repeat setting and restores it with `set_repeat_mode` at launch, every launch will emit a false RepeatModeChanged(One/All) right after the first SessionStarted. Before this round that restore would have been dropped silently; now it is reported as a user preference. Fix: add an `initial_repeat` that seeds both `repeat` and `reported_repeat`, as `initial_mode` already does for the stream mode.
- crates/tinwave-playback/src/controller/mapper.rs:332-337 — A deferred RepeatModeChanged has the session-start `ts`, not the time of the change. Its recording depends on the path: none after a load or the stale-play `track_left` (the play was already taken), and the paused track after a resume. Example: repeat-one set while track A sits paused past 30 minutes, then Next: the event has no recording, though it was made on A. The effect is small; the docs could say so.
- crates/tinwave-playback/tests/controller_thread.rs:328-330 — `ManuallyDrop::into_inner(ManuallyDrop::new(t))` does nothing (`Threaded` has no `Drop`), so a plain destructure would do. Cosmetic.

proposals:
- P: `ControllerConfig::initial_repeat` (seed the restored repeat mode without a signal) | why: stops a false RepeatModeChanged on every launch once the shell restores repeat | size: S

notes for lead: Both round-1 blocking items are fixed and tested, and the test for the non-joining exit really catches a joining teardown. Do the `initial_repeat` proposal before or with the E-01 shell wiring, whichever ticket first saves the repeat mode. The catalog note on cross-session outcomes is still yours to do (not counted here). No privacy or storage concerns: there are no new kinds, and the new debug log carries only an item id.
