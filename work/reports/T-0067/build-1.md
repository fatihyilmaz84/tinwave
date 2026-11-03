HANDOFF T-0067 (round 1)
status: done
branch: t/T-0067-exclusive-setting-signal   commit: f97fd0f
summary: ExclusiveModeToggled now comes from the listener's exclusive setting (AudioOutput::wants_exclusive) instead of stream modes. The controller reads the setting at every drain (each command and each poll) and emits {on} when it changed, in the session or deferred to after the next SessionStarted. Stream opens, reopens and fallbacks only log, so they can't emit and can't erase a deferred toggle (the T-0057 review-1 finding).

acceptance criteria:
- [x] Toggle off/on during fallback Shared emits exactly one signal each — proof: new test controller_exclusive::turning_exclusive_off_and_on_while_the_device_has_us_in_shared_is_one_toggle_each. It checks opens [48000, 44100, 48000] to show the fallback, one signal for off, nothing on a second poll, one for on, and nothing for a refused switch_device with the setting unchanged.
- [x] Fallbacks and rate reopens emit nothing; T-0057 tests pass — proof: the unchanged controller_exclusive::reopening_at_the_file_rate... and ::losing_exclusive_mode_to_a_refusing_device..., plus controller_signals::the_mode_of_a_reopened_stream_alone_never_emits_exclusive_mode_toggled. That test is rewritten: InfoOutput streams report exclusive, exclusive, then fallback while the setting never moves, and no signal appears.
- [x] Review-1 finding (mapper.rs:984-991, a fallback erasing a deferred toggle) covered — proof: new test controller_exclusive::a_toggle_made_before_a_session_survives_a_fallback_reopen. It enables the setting outside a session, switches the stream to exclusive, has the device refuse the next track's rate, and expects SessionStarted, ExclusiveModeToggled(on), PlayStarted.
- [x] fmt, clippy, nextest, deny pass — proof below.

Other tests, in controller_signals.rs:
- Rewritten to the setting: an_exclusive_setting_change_outside_a_session_is_reported_by_the_next_session.
- Rewritten to the setting: the_setting_the_controller_started_with_is_the_starting_state_not_a_toggle.
- New: an_exclusive_setting_changed_back_before_any_session_is_not_reported.

Mutations tried, all with a touched mtime:
1. Removing the setting read from Controller::drain makes 5 tests fail: both new controller_exclusive tests, the existing listener test, and 2 in controller_signals.
2. Re-adding the old behaviour (a fallback sets reported = current) makes exactly a_toggle_made_before_a_session_survives_a_fallback_reopen fail.

checks: fmt pass, clippy pass, tests 793 passed / 0 failed (6 skipped), deny pass. Stress: `--stress-count 25` over the new and exclusive tests ran 25/25 iterations clean. They wait on conditions, not sleeps.
files changed: crates/tinwave-playback/src/controller/mapper.rs, controller/mod.rs, controller/setting.rs (new), tests/controller_exclusive.rs, tests/controller_signals.rs
signals added/changed: none. ExclusiveModeToggled keeps its kind, tier and catalog row (the row already says "Listener/setting ... an automatic fallback never emits"). Only its source moves, from stream mode to setting, so there is no new volume.
ui: none
docs updated: the controller module docs in controller/mod.rs (the OutputOpened/OutputReopened table row, a new "follows the listener's setting" bullet under "Rules the mapping follows", and the "Plus ..." line).
deviations from ticket:
- output/mod.rs is untouched. Exposing the flag as a new AudioOutput trait method would need an override in output/windows.rs, which T-0066 is editing and I was told not to touch. Without it the Windows output would just return the default and never emit. Instead the controller wraps the output in controller/setting.rs (Arc<Mutex<Box<dyn AudioOutput>>>, one view for the engine and one for the controller) and reads `wants_exclusive()` through it.
- The lock is never contended: the engine opens and reopens streams only from the controller's own thread, and the decoder thread never touches the output.
- setting.rs says its SharedOutput must forward every AudioOutput method.
- Because of this there are no merge conflicts with T-0066.

Behaviours to know:
- The setting is read at each drain, so a toggle is stamped with the time of the reading, up to one poll (100 ms) after the change.
- A toggle and its undo between two readings emit nothing.
- The setting at launch is the starting state, not a change.
- The test files controller_support/mod.rs and common/mod.rs are not in scope and are untouched. InfoOutput there doesn't forward wants_exclusive, so setting-driven tests use ManualOutput directly.
- The engine only opens exclusive streams when asked (a switch_device, or a track-rate reopen from an already exclusive stream). Changing the setting alone does not reopen the stream, which is why the finding's scenario needs a switch_device first.

proposals:
- P: Expose the exclusive flag as an AudioOutput trait method, replacing the setting.rs wrapper | why: it would remove the Mutex wrapper and its "forward every method" hazard | size: S | evidence: crates/tinwave-playback/src/controller/setting.rs (a follow-up once T-0066 has merged, since it needs a windows.rs override and a ManualOutput change)

questions for lead:
- none. Process note: an unrelated python.exe (-I, running a cmake-build.cmd script, started 2-11 10:04) and a cargo process were running that are not mine. I left them alone, and nothing I started is still running.
