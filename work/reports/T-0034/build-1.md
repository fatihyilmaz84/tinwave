HANDOFF T-0034 (round 1)
status: done
branch: t/T-0034-playstarted-when-heard   commit: 8255b43
summary: PlayStarted now fires on the first PositionMark of the current track (100 ms of heard audio) instead of at TrackStarted. A track that fails, or is replaced or stopped before its first audio, leaves no PlayStarted, Skipped or Completed. The engine emits a new `EventKind::OutputOpened{device, info}` from `Engine::start`, and the mapper seeds its exclusive-mode baseline from it. `ControllerConfig::initial_mode` is removed and `initial_repeat` (P-0128) added. A session now begins with the first heard play, which is a behaviour change (see deviations).

acceptance criteria:
- [x] Failed open => no PlayStarted; normal play => exactly one PlayStarted, emitted when audio is heard — proof: tests in tests/controller_signals.rs:
  - `play_started_carries_origin_input_recording_source_and_start_position` (nothing before `hear()`, then exactly one).
  - `a_queue_of_only_a_failing_track_leaves_no_outcome_either` (no signals at all).
  - `a_failed_track_is_neither_completed_nor_skipped` (PlayStarted only for tracks 1 and 3).
  - `a_track_replaced_before_its_first_audio_is_not_a_play`.
  - `a_track_that_ends_before_its_first_position_mark_was_still_heard` (SessionStarted, PlayStarted, Completed).
  - `a_play_paused_before_it_is_heard_starts_when_it_is`.
  - `play_started_reports_where_the_play_began`.
- [x] Initial StreamInfo observable without a device switch — proof:
  - `the_engine_reports_how_each_stream_was_opened_the_first_one_too` (engine sink gets `OutputOpened{None, initial}` before any switch).
  - `the_stream_the_engine_started_with_is_the_starting_mode_not_a_toggle` (controller with no initial mode: exclusive at launch emits no ExclusiveModeToggled; switching to shared toggles off).
  - `tests/common` Harness asserts the first event of every engine is `OutputOpened`.
- [x] Pairing, rt_alloc, controller suites, concurrent stress green; fmt, clippy, tests, deny — proof: all gates below. The pairing, rt_alloc, output_rt_alloc and controller_rt_alloc binaries are unchanged and green. The controller binaries (61 tests) were run 6 times in a row, 61/61 each time.
- [x] P-0128 `initial_repeat` — proof: `a_restored_repeat_mode_is_the_starting_state_not_a_change` (initial_repeat=All, first play emits no RepeatModeChanged; setting All again is silent; One emits one).
- [x] Mapper documents the deferred RepeatModeChanged (session-start time, whichever recording is current) — in the `report_repeat` docs and catalog §8.
- [x] Nit in tests/controller_thread.rs: `ManuallyDrop::into_inner(ManuallyDrop::new(t))` replaced by a plain destructure.

checks: fmt pass, clippy pass (`--workspace --all-targets -D warnings`), tests 470 passed / 0 failed (6 ignored device/keychain tests not run), deny pass (advisories, bans, licenses, sources ok). `cargo doc -p tinwave-playback` with `-D warnings` is also clean.

files changed:
- crates/tinwave-playback/src/event.rs
- crates/tinwave-playback/src/engine.rs
- crates/tinwave-playback/src/controller/mapper.rs
- crates/tinwave-playback/src/controller/mod.rs
- crates/tinwave-playback/src/controller/types.rs
- crates/tinwave-playback/tests/common/mod.rs
- crates/tinwave-playback/tests/controller_support/mod.rs (new `Rig::hear()`, `Rig::with_repeat`)
- crates/tinwave-playback/tests/controller_signals.rs
- crates/tinwave-playback/tests/controller_sessions.rs
- crates/tinwave-playback/tests/controller_thread.rs
- docs/guides/signal-catalog.md

signals added/changed: no new kinds. PlayStarted timing changes: ts is the first-mark time (the end time for the short-track case), `pos_ds` is still where the play began, tier T0, volume unchanged (one per heard play, fewer than before). PreviousPressed is now emitted at the press, before PlayStarted, and still opens a session if none is open. The catalog is updated in the same change: the PlayStarted row, the SessionStarted trigger wording, and three notes under §8 "Session boundaries".
ui: none
docs updated: docs/guides/signal-catalog.md, plus the controller module docs (event-mapping table and rules) in crates/tinwave-playback/src/controller/mod.rs.

deviations from ticket:
- The ticket says nothing about sessions. I made a session open on the first heard play, via `ensure_session` inside the PlayStarted emission. Opening it at TrackStarted would have left a SessionStarted/SessionEnded pair around a failed track, contradicting "a failed track leaves nothing". `SessionEnded.tracks` now counts heard plays. Resume of a never-heard play no longer opens a session with origin Resume; the PlayStarted opens it with the real origin.
- Outcome rule for an unheard play: Replaced or Stopped before the first mark emits no Skipped/Completed, so an outcome always follows its start. Finished emits PlayStarted then Completed (a track under 100 ms, or a seek to the very end). Skips within the first 100 ms of heard audio are therefore not recorded; this is documented in the catalog and the controller docs.
- Pause, seek and volume signals within a play's first 100 ms can precede its PlayStarted when a session is already open. Documented in the catalog and the controller docs.
- Removing `ControllerConfig::initial_mode` is a breaking change to a public struct. Nothing outside the crate's tests used it (grep of apps/ and crates/). `Controller::start` now drains once, so the starting mode is known before any command.
- Tests were updated for the new timing: many now call `rig.hear()`, and one new test helper was added. Expectations changed in controller_signals, controller_sessions and controller_thread. controller_thread's stress test now resumes and hears the track at the end, and waits for the mapped PlayStarted with a bounded loop, so `session_starts == 1` is not flaky. `quitting_does_not_wait_for_a_decoder_stuck_in_a_read` now expects no signals, since nothing was ever heard.

proposals:
- P: Expose the current StreamInfo / stream mode to the shell (e.g. `Controller::stream_info()` and `PlaybackStatus.stream_mode`, or a field on ControllerHandle) | why: the mapper now sees OutputOpened/OutputReopened, but the shell can only observe them through signals; the Diagnostics screen needs the live mode and fallback reason | size: S | evidence: crates/tinwave-playback/src/controller/mapper.rs (`output_opened`/`output_reopened` keep only the mode) and the missing getter on ControllerHandle
- P: Decide whether a skip before the first 100 ms of heard audio should count as a rejection signal | why: rapid next-next-next is now silent, which is conservative but loses a weak "didn't want this" signal | size: S | evidence: mapper.rs `track_left` returns early for an unheard Replaced

questions for lead:
- none

---
Lead (2026-11-02):
- Session opens on the first heard play: accepted (consistent with "a failed track leaves nothing").
- Unheard Replaced/Stopped emits no Skipped/Completed: accepted for v1. A track the listener never heard is navigation,
  not a rejection; a −1.0 skip weight on it would teach taste something false. The reviewer checks it against the hard
  rule ("every interaction that changes playback emits a signal").
- Removing `ControllerConfig::initial_mode`: accepted (no users outside the crate's tests).
- Proposals: P-0135 (expose StreamInfo to the shell) parked → first controller shell-wiring / Diagnostics ticket;
  P-0136 (pre-hear skips as a weak rejection, e.g. a skip-burst counter) parked → taste engine (Phase 7).
