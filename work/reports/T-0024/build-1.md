HANDOFF T-0024 (round 1)
status: done
branch: t/T-0024-playback-controller   commit: ed6d064 (feature 9ec6a59 + merge of main 58f6d97; Cargo.lock auto-merged, --locked build clean)
summary: `Controller` (synchronous core owning the Engine) and `ControllerHandle` (same on its own thread) map every PlaybackEvent to T0 signals on the real bus with no wildcard arm (a new EventKind won't compile until mapped); listening sessions on the injected clock; 600 ms volume and 2 s seek coalescers; status read API; `BoundedSink` (bounded, ordered, reserved budget for critical events, P-0042); `StreamInfo` in `OutputReopened` (P-0082). No tinwave-context dependency. Mapping in controller/mapper.rs, logic documented in controller/mod.rs.

acceptance criteria:
- [x] Emission test per mapped kind — tests/controller_signals.rs (26 tests: PlayStarted, Completed, Skipped, SeekForward, SeekBack, Paused, Resumed, VolumeChanged, PreviousPressed, RepeatModeChanged, ExclusiveModeToggled) and tests/controller_sessions.rs; real Engine on ManualOutput, real SignalBus, ManualClock.
- [x] Failed ⇒ neither Completed nor Skipped; seek-to-end ⇒ SeekForward + Completed; seek gesture never merges across a track change — `a_failed_track_is_neither_completed_nor_skipped`, `a_queue_of_only_a_failing_track_leaves_no_outcome_either`, `seeking_to_the_very_end_is_a_seek_and_a_completion` (SeekForward before Completed), `a_seek_gesture_never_merges_across_a_track_change`; removing the flush_all calls fails 3 of these.
- [x] Session 29:59 vs 30:00 — `silence_of_29_59_keeps_the_session_and_30_00_ends_it`, `resuming_after_29_59_continues_the_session`, `resuming_after_exactly_30_minutes_is_the_first_play_of_a_new_session`, `the_end_of_the_queue_starts_the_silence`, `pause_then_quit_ends_the_session_at_the_quit`, session.rs unit tests.
- [x] Disabled category ⇒ zero signals — `disabling_a_category_constructs_nothing_of_it`, `with_every_category_off_nothing_is_constructed_and_playback_goes_on` (no seq spent, queued_total 0, playback continues).
- [x] Critical events never dropped by the bounded sink — tests/controller_sink.rs (50 tracks × 5 000 marks; two producers, 300 000 marks; newest marks kept, order kept; critical overflow counted).
- [x] Zero callback allocations; pairing green; no wall-clock in domain code — rt_alloc, output_rt_alloc, pairing pass; new controller_rt_alloc; controller_random seeded walk with clock jumps across 30 min; no Instant/SystemTime/SystemClock in src/controller (recv_timeout scheduling only).
- [x] fmt, clippy, tests, deny; default run opens no device.

checks: fmt pass, clippy (--locked) pass, tests 448 passed / 0 failed, deny pass; stress: 16 playback/controller binaries ×4 rounds + controller_thread/controller_signals ×12 + random walk at 150 seeds, no failures.
files changed: crates/tinwave-playback/{Cargo.toml (tinwave-signals), src/lib.rs, src/event.rs + src/engine.rs (`info: StreamInfo` in OutputReopened only), src/controller/{mod,mapper,session,sink,runtime,types}.rs, tests/engine.rs (adapted), tests/controller_{signals,sessions,sink,thread,random,rt_alloc}.rs, tests/controller_support/mod.rs}; Cargo.lock (+1).

mapping:
- TrackStarted{Requested} → PlayStarted (origin from LoadIntent::Select or the entry's QueueOrigin for Next/Previous; input from the command; pos_ds start frame) + PreviousPressed for LoadIntent::Previous; SessionStarted first if no session.
- TrackStarted{Advance} → PlayStarted (input Auto; Manual/Shuffle→Queue, Radio→Radio, Album→Album, Playlist→Playlist).
- TrackLeft{Finished} → Completed(listened_ds, paused_ds); {Replaced} → Skipped(fraction, input of the replacing command, section Unknown, queue_origin; pos_ds) or Completed at ≥ 97 %; {Stopped} → Completed only at ≥ 97 %; {Failed} → nothing.
- Paused → Paused{input}; Resumed → Resumed{pause_ds, input} (after ≥ 30 min silence: SessionEnded + SessionStarted{Resume} first); only when a track is current.
- Seeked → seek coalescer (2 s, sum-of-jumps) → SeekForward/SeekBack; flush_all on every TrackStarted/TrackLeft and session end.
- VolumeChanged → volume coalescer (600 ms) → VolumeChanged; only inside a session.
- OutputReopened{info} → ExclusiveModeToggled{on} when the mode changes; fallback reason logged at warn.
- set_repeat_mode → RepeatModeChanged once per change (record + report only).
- Sessions: first play after ≥ 30 min silence / after 30 min silence on the injected clock; minutes = start to last audible moment; tracks = TrackStarted count; id from IdGenerator; bus.set_tz_offset + TimezoneSource::session_started hook at each start.
- Logged only: QueueEnded, TrackFailed, SeekFailed, CorruptPackets, Underrun, OutputLost; PositionMark ignored. Section UNKNOWN, ms_since_section_start 0.
not mapped (why): RateChanged (no rate until T-0027; hook documented, exhaustive match forces it); Replayed (history/repeat-one, T-0023); Queue* (T-0023); Favorited/Unfavorited/Rated/Playlist*/MetadataEdited/ArtworkChanged/Search*/ViewOpened/WindowState/UI counters (library/UI); SystemVolumeChanged/MuteToggled/OutputDeviceChanged (tinwave-context); EqChanged/CrossfadeChanged (no feature); RadioFeedback/ExplanationOpened (Phase 7); PerceivedLevel/RateSession (derived).

deviations: `Controller::load` takes `QueueEntry` (Track + RecordingId + SourceId + duration_ms > 0 + QueueOrigin); `command_pending()`, `position()`/`volume()` passthroughs, `FixedTimezone`, `TimezoneSource` trait.
proposals:
- P: PlayStarted carries no volume level (PerceivedLevel can't be derived) | size: S
- P: Context snapshot attachment (SignalEvent.ctx; a ContextTagger seam) | size: M
- P: Engine start publishes the initial StreamInfo | size: S
- P: Defer PlayStarted until audio is heard, or mark failed plays | size: M
questions:
- PreviousPressed pos_ds: returned-to track's start, or press position in the left track?
- Skipped on a play that outlived its session: dropped — acceptable?
- Stop below 97 % emits nothing — OK, or a Skipped variant?
- RepeatModeChanged in the controller or T-0023?

---
Lead answers (2026-11-02): Q2 changed — stale outcomes are emitted in a new session (SessionStarted → outcome → next
PlayStarted), amend before review. Q1, Q3, Q4 and the deviations accepted as built. Proposals: ctx → ContextHub (E-01),
PlayStarted-when-heard + initial StreamInfo → T-0034, PerceivedLevel volume → E-04 taste (P-0094..P-0097).
