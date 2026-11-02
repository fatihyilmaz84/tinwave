HANDOFF T-0031 (round 1)
status: done
branch: t/T-0031-engine-boundary-followups   commit: a16046f (on top of main 082863f)
summary: A seek or device-switch resume at exactly a track's end finishes the track and advances; position() no longer names an unannounced track; WasapiOutput doc note added. Each fix has a deterministic test in tests/boundary.rs that fails with that fix reverted.

items:
1. reader.rs seek: frame == length is the end of the track (positioned at end; next next_chunk reports EOF; decoder opens the next item) => TrackLeft{A, frame: length, Finished} + TrackStarted{B, Advance}; only frame > length is SeekOutOfRange. Tests: switch_device_exactly_at_a_track_end_finishes_the_track_and_advances (gated opener; no SeekFailed/TrackFailed/Failed; B bit-exact from 0; pairing holds); seeking_to_exactly_the_end_finishes_the_track_and_advances (wav, flac, alac; Seeked emitted, A Finished); seeking_to_the_end_of_the_last_track_ends_the_queue_as_finished; seeking_past_the_end_is_still_a_failure (length + 1). Behaviour change: seek to exactly length used to be SeekFailed; failures.rs seek-past-end and GaplessInfo trim tests now use length + 1.
2. position() with no current track returns None (documented on Engine::position and event.rs): only what a command announced, never an unannounced mark; None after stop, after queue end (even with an appended track decoding), before any load. Tests: no_event_position_names_an_unannounced_track (Paused/VolumeChanged/Resumed carry None; after announcement names B); a_failed_first_choice_does_not_leak_the_next_tracks_position.
3. WasapiOutput doc: the post-claim wait in open has no timeout of its own, bounded only by the driver (next to the drop-joins note).

before-the-fix: both reverted => 5 of 6 boundary tests fail (past-end passes, as it should); reader only => switch, seek-to-end, last-track fail; engine only => the two position tests fail.
checks: fmt pass; clippy pass; tests 364 passed / 0 failed / 6 ignored (default run opens no device; ignored device tests not run); deny pass; pairing default + TINWAVE_PAIRING_SEEDS=256 pass (86 s); rt_alloc + output_rt_alloc pass; stress 14 playback executables: 588 (6 x 7) + 840 (12 x 5) = 1428 processes, 0 failures.
files changed: crates/tinwave-playback/src/{reader.rs, engine.rs, event.rs (docs), output/windows.rs (doc)}, tests/boundary.rs (new), tests/failures.rs (two cases adjusted)
signals: none. For T-0024: an exact-boundary completed play is Finished, never Failed; no Paused/Resumed/VolumeChanged position names a track without TrackStarted.
deviations/proposals/questions: none
