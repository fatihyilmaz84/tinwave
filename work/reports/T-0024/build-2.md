HANDOFF T-0024 (round 2)
status: done
branch: t/T-0024-playback-controller   commit: 835152a (on top of 660dcf0; no rebase needed)
summary: Both blocking findings are fixed and the five small items are done. Quitting no longer waits for the decoder thread. A preference change made while no session is open is now reported right after the next SessionStarted.

findings addressed:
1. (blocking) Exit must never wait for the decoder.
   - New non-joining `Controller::close()`: it stops playback, drains, and calls `mapper.finish` (so SessionEnded is still emitted), then drops the Engine without joining the decoder.
   - `shutdown()` still joins and is kept for tests.
   - `ControllerHandle`'s controller thread now ends with `close()`, so `drop` and `shutdown` return promptly.
   - Test: `quitting_does_not_wait_for_a_decoder_stuck_in_a_read` in `tests/controller_thread.rs`. It uses a `GatedOpener` whose first read never returns, waits until the decoder is blocked, then drops the handle on a helper thread and requires it to return within 5 s. It also asserts [SessionStarted, PlayStarted, SessionEnded] on the way out.
   - I temporarily switched back to `shutdown()` and the test failed with "dropping the handle waited for the decoder", so it detects the bug.
2. (blocking) Preference changes outside a session are lost.
   - The mapper now keeps `reported_repeat` and `reported_mode` next to the current values. A change with no open session is only recorded. `ensure_session` calls `report_preferences`, which emits `RepeatModeChanged` and/or `ExclusiveModeToggled` right after `SessionStarted` if the value still differs from the last one reported.
   - A change made back to the reported value before any session emits nothing.
   - Tests in `tests/controller_signals.rs`:
     - `a_repeat_mode_set_with_nothing_loaded_is_reported_by_the_next_session` expects SessionStarted, RepeatModeChanged(One), PlayStarted.
     - `a_repeat_mode_changed_back_before_any_session_is_not_reported`.
     - `a_repeat_mode_set_after_a_session_ended_is_reported_by_the_next_one` (covers the over-30-minute case).
     - `an_exclusive_mode_change_outside_a_session_is_reported_by_the_next_session` expects SessionStarted, ExclusiveModeToggled(on), PlayStarted.
3. Load race: I chose to log at `debug`, not to keep old entries. Merging old and new tables risks `QueueItemId` collisions across queues. The comment at the log site says the case cannot be told apart from a queue-layer bug.
4. Seek with an expired session: documented next to the volume rule in the "Gestures need a session" bullet of the `controller/mod.rs` docs. No behaviour change.
5. Docs:
   - `switch_device` can block 100 ms or more (synchronous output open); the shell should call it off the UI thread. Stated in the `controller/mod.rs` docs, the `Controller::switch_device` doc and the `ControllerHandle` doc in `runtime.rs`.
   - `TimezoneSource` implementations must not block: added to the trait docs in `types.rs`.
   - Also documented: the non-joining exit, and the preference-reporting rule.

Not done: the signal-catalog note about cross-session outcomes (review item and proposal) is out of this ticket's scope; I did not touch docs/ or work/.

checks: fmt pass, clippy `-D warnings` pass (`--workspace --all-targets --locked`), cargo doc for the crate has no warnings, tests 455 passed / 0 failed (workspace; device tests stay ignored and were not run), deny pass.
stress: 16 playback and controller test binaries (controller suites, pairing, rt_alloc, output_rt_alloc, engine, seek, boundary, failures, gapless, append, position) ran concurrently ×4 rounds with no failures. controller_thread and controller_signals ran ×12 in parallel and the random walk ran at 100 seeds, also clean.
files changed this round: crates/tinwave-playback/src/controller/{mod,mapper,runtime,types}.rs, crates/tinwave-playback/tests/controller_signals.rs, crates/tinwave-playback/tests/controller_thread.rs
signals added/changed: none (no new kinds)
deviations from ticket: none
proposals: none new
questions for lead: none
