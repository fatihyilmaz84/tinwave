HANDOFF T-0024 (round 1, amended)
status: done
branch: t/T-0024-playback-controller   commit: 660dcf0
summary: A Skipped or Completed for a play that outlived its session is no longer dropped. At 30:00 of pause plus Next or Select, the signals come out as SessionEnded, SessionStarted, Skipped, PlayStarted. SessionStarted takes its origin from the command: the chosen origin for Select, or the next or previous entry's QueueOrigin mapped to a PlayOrigin for Next and Previous. Q1, Q3 and Q4 stay as built. The rule is documented in the `controller/mod.rs` module docs.
checks: fmt pass, clippy -D warnings pass, tests 450 passed / 0 failed (workspace, device tests ignored), deny pass
stress: 16 playback and controller test binaries ran concurrently for 4 rounds, controller_sessions ran 12 in parallel, and the random walk ran at 100 seeds, all clean
files changed: crates/tinwave-playback/src/controller/mapper.rs, crates/tinwave-playback/src/controller/mod.rs, crates/tinwave-playback/tests/controller_sessions.rs
signals added/changed: none
deviations from ticket: none beyond those you accepted
questions for lead: none

Notes from the builder:
- mapper.rs `track_left` opens a session with the command's origin before it emits a Completed or a Replaced-leave Skipped; mod.rs Next/Previous derive that origin from `entries[index].origin`.
- Exception: a `stop` in the last 3 % of such a stale play has no command origin, so that one Completed is still not kept (documented).
- Tests: the old drop-asserting test is replaced by three (30:00 + Next ⇒ SessionEnded, SessionStarted(Playlist), Skipped, PlayStarted; 29:59 + Next ⇒ same session; 30+ min + Select ⇒ SessionStarted(Search)).
