REVIEW T-0034 (round 1)
verdict: CHANGES_REQUESTED
checks (run by reviewer): fmt pass; clippy pass (`--workspace --all-targets -D warnings`); tests: first full `cargo nextest run --workspace` gave 469 passed / 1 FAILED (`controller_thread::concurrent_callers_lose_nothing_and_the_engine_stays_consistent`, controller_thread.rs:322, `left: 0, right: 1`, 21.9 s), then 4 more full runs gave 470/470. The controller_thread, controller_signals and controller_sessions binaries passed 8 of 8 runs on their own. The stress test alone passed 168 of 168 (up to 32 processes at once). deny pass (advisories, bans, licenses, sources ok).

acceptance criteria:
- [ ] Failed open ⇒ no PlayStarted; normal play ⇒ exactly one PlayStarted, when heard — problem: covered for open failures and normal plays by `a_queue_of_only_a_failing_track_leaves_no_outcome_either`, `a_failed_track_is_neither_completed_nor_skipped`, `play_started_carries_origin_input_recording_source_and_start_position`, `a_track_replaced_before_its_first_audio_is_not_a_play`. But a track that opens and then decodes zero frames still emits SessionStarted + PlayStarted + Completed (blocking finding 2).
- [x] Initial StreamInfo observable without a device switch — verified by: `the_engine_reports_how_each_stream_was_opened_the_first_one_too` (controller_signals.rs, `OutputOpened{None, initial}` before any switch), `the_stream_the_engine_started_with_is_the_starting_mode_not_a_toggle`, and the Harness check in tests/common/mod.rs:202.
- [ ] Pairing, rt_alloc, controller suites, concurrent stress green; fmt, clippy, tests, deny — problem: the concurrent stress test failed in 1 of my 5 full-workspace runs (blocking finding 1). pairing, rt_alloc, output_rt_alloc and controller_rt_alloc are green and unchanged.
- [x] P-0128 `initial_repeat` seeds repeat and reported_repeat — verified by: mapper.rs:163-164 and `a_restored_repeat_mode_is_the_starting_state_not_a_change`.
- [x] Deferred RepeatModeChanged documented — verified by: mapper.rs:347-353 and signal-catalog.md §8 (session-start time, recording current then).
- [x] ManuallyDrop nit — verified by: controller_thread.rs:358-360, now a plain destructure.

blocking findings:
1. crates/tinwave-playback/tests/controller_thread.rs:287-298 and :109-123 — the stress test is flaky, and its wait loop hides the cause.
   - What happens: `hear()` renders exactly 4,800 "real" frames and then stops rendering. The loop then waits up to 30,000 × 200 µs for a PlayStarted. It never asserts that one arrived; it just falls through to `assert_eq!(session_starts, 1)`.
   - Observed: 21.9 s run, `session_starts == 0`. The loop timed out, so no PlayStarted was ever emitted, before or after `hear()`.
   - Why it matters: with this ticket the first PositionMark decides whether a heard play gets PlayStarted, Skipped and Completed at all. "A heard play never got its mark" therefore has to be ruled out, not retried away. I could not reproduce it alone (168/168), so the root cause is open. It may be a test-timing problem or a real case where marks stop.
   - Fix: (a) make the wait render and poll until PlayStarted is seen, and panic on timeout with `status()`, `position()`, `command_pending()` and `sink_stats()` in the message; (b) rerun the full workspace until it reproduces and find the cause; (c) apply the same "render until seen" pattern in `dropping_the_handle_ends_the_session`, which uses the same fixed-count `hear()`.
2. crates/tinwave-playback/src/controller/mapper.rs:482-491 — the new "Finished ⇒ heard" path has no guard.
   - Mechanism: a file that opens but decodes no audio still ends in `TrackLeft{Finished}` with frame == start frame. Two ways: `next_chunk` returns `Ok(false)` at the first packet (an `UnexpectedEof` — truncated download or rip — or an empty data chunk, reader.rs:215-217); or every packet is a `DecodeError`, so all are skipped (reader.rs:227-229) and decoder.rs:390 calls `finish_item`, not failed.
   - Effect: the mapper emits SessionStarted + PlayStarted + Completed for a track nobody heard — the case the ticket set out to remove — and the Completed teaches taste a false positive.
   - Fix: treat an unheard `Finished` as a play only if audio actually advanced. Have the mapper record the start frame of the current stream (TrackStarted frame, updated on `Seeked.to_frame`) and require leave frame > that frame. Add a test with a header-only or zero-frame WAV that expects no signals.
3. Hard-rule check (CLAUDE.md, "every user interaction that changes playback … MUST emit a signal") — mapper.rs:564-566 with `send` at :694, and the test `a_play_paused_before_it_is_heard_starts_when_it_is` (asserts empty after Pause(Mouse)/Resume(Mouse)).
   - What is lost: for an unheard play with no session open, Pause, Resume and Seek (and Volume) are now dropped. On main they were recorded, because TrackStarted opened the session.
   - Concrete case: at launch the shell restores the last track paused at its saved position. The listener drags the seek bar, then presses Play on a media key. Seek and Resume both emit nothing; the PlayStarted that follows carries the load's input (e.g. Auto), not MediaKey. This is the most common way a desktop session starts, it is a regression, and the lead's acceptance covered only Replaced/Stopped outcomes.
   - Fix: when Paused/Resumed/Seeked/VolumeChanged arrives with a user `cx.input` for an unheard play and no session is open, call `ensure_session(at, pending.origin)` first, then emit as before. This does not bring back sessions made only of failed tracks: the decoder opens and fills while paused, so an open failure is reported as `TrackLeft{Failed}` before the user can resume. Update that test and the "can precede its PlayStarted" docs (mod.rs:62-64, catalog §8).
   - Full list of interactions that now emit no signal: (i) Select/Next on a track that fails before audio — ticket mandates, not blocking; (ii) Next/Select/Previous replacing a track never heard or within its first ~100 ms — lead accepted (P-0136), navigation, not blocking; (iii) Stop of an unheard play — already silent on main before the last 3 %, not new; (iv) Pause/Resume/Seek/Volume of an unheard play outside a session — new, blocking.

design check: n/a (no UI)

non-blocking findings:
- crates/tinwave-playback/src/controller/sink.rs:13-16 — docs say "only the newest [mark] is ever useful", and overflow removes the oldest mark. The mapper now relies on the first mark of each track: if coalescing ever dropped a track's only mark before a Replaced, that heard play's PlayStarted and Skipped would vanish. Unreachable today (the controller drains after every `engine.poll`). Update the docs, or never coalesce away a mark whose item differs from the next mark's.
- crates/tinwave-playback/src/engine.rs:503-521 — a gapless Continuation does not reset `grid_base`/`last_grid`, so an auto-advanced track's first mark (and its PlayStarted) can come 0–100 ms into it, not after 100 ms as the catalog row, mod.rs:56-57 and mapper.rs:436 say. Fix the wording ("the first position mark that names it").
- crates/tinwave-playback/src/controller/mapper.rs:426-432 — a Previous to a track that then fails opens a session with `tracks = 0` (SessionStarted + PreviousPressed only), contradicting mod.rs:72-73 and catalog §8 "a track that fails to open does not open one". Document the exception or defer the session.
- crates/tinwave-playback/tests/controller_random.rs:166-172 — the pairing walk never asserts that any PlayStarted happened, and does not catch a second PlayStarted before an outcome. Add `assert!(plays > 0)` and "no PlayStarted while a play is open without an outcome".

proposals:
- P: Assert "every heard play gets exactly one PlayStarted" in the random walk (render-count based) | why: the first mark is now load-bearing for all T0 play signals | size: S

notes for lead:
- Verified intact: T-0024 guarantees (non-joining close test still detects a joining teardown; SessionEnded on drop still covered; 29:59/30:00 tests kept and extended; preferences still reported after the next SessionStarted, assertions not weakened; `initial_repeat` silent). `OutputOpened` is emitted on the control thread from `Engine::start` into the pre-allocated BoundedSink, not on the audio callback; rt_alloc binaries green. No consumers of `initial_mode` or `EventKind` outside the crate.
- Root cause of finding 1 is unknown. Have the builder reproduce it before merge rather than just hardening the wait.

---
Lead (2026-11-02): all three blocking findings stand; fixes as the reviewer proposes (finding 3: open the session on a
user Pause/Resume/Seek/Volume of an unheard play). Round 2 also takes the four non-blocking items and the proposal
(P-0137 → merged into T-0034 round 2). Finding 1 needs a root cause, not only a hardened wait.
