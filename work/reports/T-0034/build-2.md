HANDOFF T-0034 (round 2)
status: done
branch: t/T-0034-playstarted-when-heard   commit: 7d3c1ad
summary: All three blocking findings and the four non-blocking items are fixed. The stress-test flake was a test timing bug, not a missing mark in the engine, and I reproduced it. A track that "finishes" with no audio is no longer a play. A listener's pause/resume/seek/volume on an unheard play now opens the session first.

Root cause of finding 1 (reproduced):
- `hear()` rendered a fixed 4,800 real frames and then stopped rendering. Real frames returned by the callback can still come from the previous stream, because the decoder has not yet carried out the last Load/Seek, so those frames are flushed or stale.
- The final stream therefore never got its own 100 ms, and no mark was due. Earlier streams in the stress test were restarted by the workers' commands before reaching 100 ms either, so the whole run had no PlayStarted and `session_starts == 0`.
- Reproduction: a temporary copy of the old test with diagnostics, run as 120 parallel processes, failed about 3 times per round of 120. Typical output: `at_hear_end … command_pending: true, position frame 0` (stale frames counted; flush not yet acknowledged), or `command_pending: false` with position 3,840 frames (80 ms: the new stream had been rendered for only 80 ms); `kinds []`.
- Ruled out "a heard play never got its mark": at every failure the final stream had less than 100 ms of audio rendered, and a few seconds later the position and pending state were consistent. No mark was missing for a stream that had 100 ms.
- Fix: `play_until_started` keeps rendering and collecting signals until a PlayStarted is seen. On timeout it panics with `status()`, `diagnostics()` (position, command_pending, sink stats), buffered frames and the signal kinds. Used in the stress test and in `dropping_the_handle_ends_the_session`.
- Added `ControllerHandle::diagnostics()` and `Controller::diagnostics()`, returning `ControllerDiagnostics{position, command_pending, sink}`. The handle snapshot is refreshed after each controller step, next to status.
- After the fix: the stress and drop tests passed 360 runs at 120 in parallel, and the whole controller_thread binary passed 240 runs at 120 in parallel.

acceptance criteria / findings:
- [x] 1. Flaky stress test: cause found and reproduced as above; waits are now render-and-poll with a diagnostic panic.
- [x] 2. Unheard Finished guard: the mapper tracks `from_frame` per play (TrackStarted frame, updated on `Seeked.to_frame`); an unheard Finished is a play only if the leave frame is greater than `from_frame`. New tests `a_file_that_opens_but_has_no_audio_is_not_a_play` (zero-frame WAV, then a normal track) and `a_seek_to_the_very_end_before_any_audio_is_not_a_play`; both fail when the guard is removed.
- [x] 3. Hard rule: `open_session_for_unheard` runs on Paused, Resumed, Seeked and VolumeChanged when the play is unheard, no session is open and the input is not Auto. It calls `ensure_session(at, play origin)` first, then the event is emitted as before. `Auto` means the shell's own command and is not the listener's act. Tests: `a_play_paused_before_it_is_heard_starts_a_session_with_the_pause` (SessionStarted, Paused, Resumed, PlayStarted); `a_track_restored_paused_then_sought_and_resumed_records_what_the_listener_did` (Auto restore silent; Mouse seek and MediaKey resume recorded); `a_volume_change_on_a_play_not_yet_heard_is_recorded`. Docs updated in mod.rs and catalog §8.
- [x] Non-blocking: `BoundedSink` now removes the oldest mark that is followed by a mark of the same track (or by the incoming one); only if every queued mark names a different track does it fall back to the plain oldest. Test `the_first_mark_of_a_track_is_never_coalesced_away_by_marks_of_another_track`. Existing coalescing test unchanged and green.
- [x] Non-blocking: gapless wording is now "the first position mark that names it, which can come sooner than 100 ms in after a gapless hand-off" in the catalog row, mod.rs and mapper.rs.
- [x] Non-blocking: Previous to a track that then fails — documented the exception: the press opens a session with `tracks = 0` (catalog §8, mod.rs Sessions). Test `previous_to_a_track_that_fails_still_records_the_press_and_its_session`.
- [x] Non-blocking and P-0137: the random walk asserts total `plays > 0`. Each seed ends with an epilogue (resume, stop, load a clean track, `hear()`) asserting exactly one PlayStarted for it, and still exactly one after another second of play. Not added: "no second PlayStarted while a play is open without an outcome" in the random walk — Stop, and a Stopped play before 97 %, legitimately leave a heard play without an outcome (false positives); it is asserted in the concurrent stress test (Next-only commands), where it holds.

checks: fmt pass; clippy pass (`--workspace --all-targets -D warnings`); `RUSTDOCFLAGS=-D warnings cargo doc -p tinwave-playback` pass; deny pass; full-workspace `cargo nextest run --workspace`, 7 runs: 6 × 476 passed / 0 failed / 6 skipped; 1 × 475 passed / 1 failed — `tinwave-db::storage_growth ninety_days_of_a_heavy_listener…` took 154 s (normally ~30 s), crate untouched; passed alone (69/69) and in 8 other runs. Looks like host load.

files changed: crates/tinwave-playback/src/controller/{mapper,mod,sink,types,runtime}.rs, crates/tinwave-playback/src/lib.rs, crates/tinwave-playback/tests/{controller_signals,controller_thread,controller_random,controller_sink}.rs, docs/guides/signal-catalog.md

signals added/changed: no new kinds. PlayStarted semantics refined as above. Pause, Resume, Seek and Volume on an unheard play are recorded again (restores main's behaviour). Catalog §4 and §8 updated. No storage impact.
ui: none
docs updated: docs/guides/signal-catalog.md; controller module docs in mod.rs; sink.rs docs.
deviations from ticket: none beyond round 1. New public `ControllerDiagnostics` and `diagnostics()` accessors (requested by the reviewer for failure messages; basis for the Diagnostics screen).

proposals:
- P: Make `PlaybackStatus` or `ControllerDiagnostics` carry the current `StreamInfo` (mode, fallback reason) | why: the Diagnostics screen needs it | size: S | evidence: types.rs `ControllerDiagnostics`, mapper.rs `output_opened`
- P: A `Rig::render_until_heard` test helper that waits for the pending command before counting frames | why: `hear()` in the sync rig has the same stale-frame hazard, avoided only by `render_exact` calling `settle()` first | size: S | evidence: tests/controller_support/mod.rs `hear`

questions for lead:
- none

---
Lead (2026-11-02): storage_growth timeout under load is known → T-0047. Proposals: StreamInfo in diagnostics merged →
P-0135 (parked, shell wiring / Diagnostics); `Rig::render_until_heard` → P-0143 merged → T-0023 (next playback ticket
that uses the rig).
