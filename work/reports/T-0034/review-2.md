REVIEW T-0034 (round 2)
verdict: CHANGES_REQUESTED
checks (run by reviewer): fmt pass; clippy pass (`--workspace --all-targets -D warnings`, exit 0); tests: 4 full `cargo nextest run --workspace` runs. Three gave 476/476 passed (6 skipped). One gave 475 passed plus 1 failure: `tinwave-db::storage_growth ninety_days…`, a 104 s wall-clock timeout under load, the known T-0047 issue, not counted. controller_thread binary: 192 runs at 32 in parallel (6 rounds), 0 failures. Stress and drop tests alone: 512 runs at 64 in parallel (8 rounds), 0 failures. deny pass.

acceptance criteria:
- [x] Failed open ⇒ no PlayStarted; normal play ⇒ exactly one PlayStarted, when heard — round-1 tests plus `a_file_that_opens_but_has_no_audio_is_not_a_play` and `a_seek_to_the_very_end_before_any_audio_is_not_a_play`; removing the `frame <= play.from_frame` guard in a scratch copy fails both. The random-walk epilogue asserts exactly one PlayStarted per clean heard play, plus `plays > 0`.
- [x] Initial StreamInfo observable without a device switch — unchanged, green.
- [x] Pairing, rt_alloc, controller suites, concurrent stress green; fmt, clippy, tests, deny. Flake root cause: the builder's explanation is convincing and matches round 1 (`session_starts == 0` after 21.9 s): the workers' last steps often leave the track paused or with a command pending; the old `hear()` counted up to one block of stale frames, so the final stream got under 100 ms and the wait loop timed out without rendering. `play_until_started` (controller_thread.rs:112-134) renders and polls with a diagnostic panic; used by the stress test and `dropping_the_handle_ends_the_session`. `ControllerDiagnostics` is a Copy snapshot built on the controller thread behind its own mutex; nothing on the audio callback is touched; it exposes only values already public.
- [x] P-0128 items intact.

blocking findings:
- crates/tinwave-playback/src/controller/mapper.rs:549-556 (new `open_session_for_unheard`, used at :606 Seeked and :643 VolumeChanged) and :426-431 (the round-1 Previous branch).
  - Defect: a session opened while playback is paused is marked audible. `Session::new` starts with `silence_since = None` (session.rs:28-35). Only the Pause path calls `set_audible(false)` after opening. Seek and volume never do. The Previous branch calls `set_audible(!paused)` *before* `ensure_session`, so it is a no-op there; main did it after (main mapper.rs:387 then :398), so this part is a round-1 regression.
  - Effect: such a session never ends by silence. A play pressed hours later joins it, and `SessionEnded.minutes` counts all the paused time — breaks catalog §8 (sessions end after 30 min of silence; the first play after opens a new one).
  - Reproduced (scratch probe): restore paused (`Auto`), `set_volume(0.5, Mouse)`, 120 polled minutes, `resume(MediaKey)`, `hear()` → `[SessionStarted, VolumeChanged, Resumed, PlayStarted, SessionEnded]`, one session, `minutes=120`. Same with `seek_ms(5_000, Mouse)` and with `LoadIntent::Previous` while paused. Control (pause path) correct (`minutes=0`, session opened at the resume).
  - This is the "restored paused at launch" scenario round 2 was meant to fix.
  - Fix: after opening a session, seed its silence from `self.paused` — inside `ensure_session` (`session.set_audible(at, !self.paused)`), or right after `ensure_session` in `open_session_for_unheard` and the Previous branch.
  - Tests to add: restored paused → Mouse volume (and seek, and Previous) → 30:00 polled → `SessionEnded{tracks 0, minutes 0}`; a later resume opens a new session with origin Resume. Plus the 29:59 counterpart.

design check: n/a

non-blocking findings:
- mapper.rs:558-560, 590, 606 — Pause, Resume and Seek open a session even when that signal kind is turned off, while VolumeChanged checks `bus.enabled` first (:640). Check the toggle before `open_session_for_unheard`, for consistency.
- sink.rs:116-142 — `coalescible_mark` decides "same track" by `QueueItemId` alone and ignores a TrackStarted/TrackLeft between two marks; if the same item plays twice in a row (repeat-one through a reload of the same entry), play 1's only mark can be coalesced away. Unreachable today (MARK_CAPACITY 8, drained every poll). Otherwise bounded, allocation-free, on the control thread.
- controller_thread.rs:128 — panic format string has a run of literal spaces. Cosmetic.
- Not adding "no second PlayStarted while open" to the random walk is justified.
- Verified OK: gapless wording consistent; Previous-to-failing-track exception documented and tested; `Auto` restore silent; failed-only sessions only after an explicit non-Auto act.

proposals:
- P: Session-boundary property in the random walk: no SessionEnded.minutes may exceed the audible wall time between its SessionStarted and the last audible event | why: would have caught this generically | size: S

notes for lead: single blocking issue, small fix (one line plus tests), covering all three entry points. Probes in scratchpad\t34copy\crates\tinwave-playback\tests\zz_probe.rs. Flake resolved: no recurrence in 704 parallel runs. T-0024 guarantees still green.

---
Lead (2026-11-02): round 2 failed on a one-line fix → the lead takes it over (work/README.md). Seed the new session's
silence from `paused` inside `ensure_session`; add the reviewer's tests; also the toggle-consistency item.
P-0147 (session-boundary property in the random walk) → see outcome.

Lead takeover result (2026-11-03): a1e0888. `ensure_session` seeds the new session's silence from `paused`;
`open_session_for_unheard` takes the act's kind and returns when that kind is off (Paused, Resumed, SeekForward/
SeekBack by direction, VolumeChanged). New tests in controller_sessions.rs: volume / seek / Previous while restored
paused → 29:59 keeps, 30:00 ends with tracks 0 minutes 0, a resume later opens a new session; a seek with its signal
off opens no session. Mutation (fix line removed): the three silence tests fail; restored: pass. Branch gates: fmt,
clippy, 480 passed / 6 skipped, deny. Merged 0122962.
