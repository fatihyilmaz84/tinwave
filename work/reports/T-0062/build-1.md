HANDOFF T-0062 (round 1)
status: done (two decisions for the lead below)
branch: t/T-0062-rate-session-and-status   commit: db78ad9
summary: The controller now derives one `RateSession` per heard play from the engine's `RateStats`. `PlaybackStatus.rate` is the engine's applied rate (`rate_stats().applied_rate`). `RateSession` is a derived (class D) kind that the bus refuses as an event (`SignalsError::NotAnEvent`), so it cannot be emitted on the bus. It is delivered through `Controller::take_rate_sessions()` and `ControllerHandle::take_rate_sessions()` instead. Both are new APIs the ticket did not name, so please confirm them (see questions).

acceptance criteria:
- [x] One RateSession per heard play, time-weighted rate within 0.1 % of the schedule — proof: `a_heard_play_has_one_rate_session_with_the_time_weighted_rate_of_its_schedule` (real engine, schedule 1.0/1.25/0.75/1.5/1.0, exactly 1 value, within 0.1 %, `at` = leave time, taken once, never a bus event). Also `each_play_is_measured_on_its_own`, `a_play_at_normal_speed_throughout_has_a_rate_session_of_one`, `the_handle_hands_over_rate_sessions_from_the_controller_thread`, and 10 mapper unit tests.
- [x] No RateSession with Speed off, or for an unheard/failed play — proof: `no_rate_session_with_the_speed_category_off` (also checks it is measured again once switched on), `no_rate_session_for_a_play_that_was_never_heard_or_could_not_be_played` (stop before the first mark; a missing file between two good ones), and mapper tests `a_heard_play_that_then_failed_has_none`, `a_play_left_before_it_was_heard_has_none`, `nothing_is_derived_with_the_speed_category_off`, `a_play_that_straddles_the_speed_toggle_has_none`.
- [x] PlaybackStatus reports the applied rate after a throttled change (latest value wins) — proof: `the_status_reports_the_rate_the_engine_applies_and_the_latest_change_wins`. It sets 1.25, 0.8 and 1.5 in a row. It checks that the target is 1.5 while status still says 1.0 (the stream has not restarted), then waits on the condition for status to say 1.5.
- [x] Catalog row, storage estimate and Signal Center entry consistent; growth test passes — proof: the `RateSession` row is rewritten (trigger, class D, 0 B/day in `signal_event`, estimate of about 0.1 KB/day if later kept in a play record). `catalog_sync` and the DB growth test pass in the full run (786 passed). The Signal Center has no per-kind list yet. The `Speed` category already covers `RateSession` (`settings.rs`), so no entry change is needed. §9 of the catalog is outside the ticket's scope and untouched. The row says the ticket that stores the value adds its estimate there.

checks: fmt pass, clippy pass (`--workspace --all-targets -D warnings`), tests 786 passed / 0 failed (`nextest --workspace --no-fail-fast`), deny pass (advisories, bans, licenses, sources ok)
- The first `nextest` run, with fail-fast on, stopped on `tinwave-db::signal_wipe the_writer_thread_keeps_working_while_everything_is_wiped`. I did not touch that crate. A second full run, with no fail-fast, passed everything. I believe it is load-sensitive, and the reviewer's `nextest -p tinwave-db` was running at the same time.
- Stress: 10 parallel copies of the `controller_signals` binary (55 tests each, 8 threads each) all passed. 6 parallel copies of the 13 rate tests all passed.
- A second unchanged `cargo build -p tinwave-playback` compiles nothing.

mutations tried (each reverted afterwards; I rebuilt before running the stress tests):
- `status()` returns the target `engine.rate()` instead of the applied rate: the status test fails.
- Begin snapshot ignored (measure from zero): `each_play_is_measured_on_its_own` and two mapper tests fail.
- Both Speed gates removed: three tests fail. With only the start gate removed they still pass, because the leave gate also stops it. That redundancy is deliberate.
- Failed leave not excluded: `a_heard_play_that_then_failed_has_none` fails.

files changed:
- crates/tinwave-playback/src/controller/rate_session.rs (new)
- crates/tinwave-playback/src/controller/mapper.rs
- crates/tinwave-playback/src/controller/mod.rs
- crates/tinwave-playback/src/controller/runtime.rs
- crates/tinwave-playback/src/controller/types.rs
- crates/tinwave-playback/src/lib.rs (one re-export line; outside the stated scope, see deviations)
- crates/tinwave-playback/tests/controller_signals.rs
- docs/guides/signal-catalog.md (the `RateSession` row only)

signals added/changed:
- No new kind. `RateSession` (derived, T0 by its inputs, Speed category) is now produced.
- It never reaches `signal_event`: 0 B/day there. At most 1 value per heard play. The queue holds 64 and counts what it drops in `ControllerDiagnostics.rate_sessions_dropped`.
- If a later ticket keeps it in a play record, I estimate about 1 byte per play, so about 0.1 KB/day. That is not in §9 yet.

ui: none
docs updated: the signal-catalog `RateSession` row; the controller module docs (new "`RateSession`" section and a mapping-table row)

deviations from ticket:
1. Delivery is not a bus emit. The ticket says "emit RateSession", but `SignalKind::RateSession` is `StorageClass::Derived`. `bus.emit_with` returns `NotAnEvent` for it, there is no `SignalPayload` variant, and `priority.rs` says "Derived: never emitted". Changing `tinwave-signals` is outside scope. So the controller derives the value and queues it for the consumer. New public API: `RateSession { at, recording, source, time_weighted_rate: f64 }` and the two `take_rate_sessions()` methods. The value is not quantized, because the 0.1 % criterion cannot hold at the `Rate` type's 0.01 step. Quantizing is the storing consumer's job.
2. The play's start snapshot is taken at `TrackStarted`, not at `PlayStarted`. `PlayStarted` waits for the first position mark, about 100 ms of heard audio, and a snapshot there would leave that audio out. For a requested load, `TrackStarted` comes before any audio of the new track.
3. One engine reading per drain (`Controller::drain`) is shared by every event in the batch. A gapless hand-off reports `TrackLeft(A)` and `TrackStarted(B)` together, so they share the boundary and nothing is counted twice or lost. The docs state the error honestly: about one device latency plus one poll at a play's edges (ADR 0006 note 8), exact when the rate does not change. They do not claim exactness.
4. `PlaybackStatus` lost `Eq` because it now holds an `f64`; it is still `PartialEq`. Nothing else in the workspace needed `Eq`. `ControllerDiagnostics` gained `rate_sessions_dropped`.
5. `crates/tinwave-playback/src/lib.rs` has one re-export line (`RateSession`), outside the stated scope. Without it the type cannot be named outside the crate.

decisions recorded:
- A play spent entirely at 1.0 does emit a value (exactly 1.0). The catalog row said "Per play" with no exception. The consumer needs 1.0 to tell "not rushed" from "not measured". This is written in the catalog row and the module docs.
- A play whose Speed category was off at its start has none, and neither does one switched off before it ends. A play that straddles the toggle is not measured from a moment the listener had not agreed to be measured at.
- A play with no counted audio between the snapshots has none (`time_weighted_rate_since` returns `None`).
- A heard play that is stopped mid-track, or whose session had already ended, has one. A heard play that then fails has none.
- Exclusive reopen: the counters live in the engine's shared state and are never reset, and the ratio does not depend on the stream rate. `rate_sessions_stay_right_across_exclusive_mode_reopens` (44.1 / 96 / 44.1 kHz files, four stream opens) gives three values of 1.25 within 0.1 %.

proposals:
- P: Wire `take_rate_sessions` into the play-record writer and the taste engine | why: until then the value is produced but nothing stores it; the §9 estimate belongs to that ticket | size: S | evidence: `crates/tinwave-playback/src/controller/rate_session.rs`, `crates/tinwave-db/migrations/signals/0001_init.sql` (`play.flags` / a rate column)
- P: Make derived signals a first-class concept (a `DerivedValue` type in `tinwave-signals`) | why: `PerceivedLevel` will hit the same wall as `RateSession` (bus refuses derived kinds) | size: M | evidence: `crates/tinwave-signals/src/bus.rs:739-757`, `priority.rs:96`
- P: Make the `tinwave-db` signal_wipe writer-thread test robust under parallel load | why: it failed once in a fail-fast run and passed on rerun | size: S | evidence: `tinwave-db::signal_wipe the_writer_thread_keeps_working_while_everything_is_wiped` (16.9 s in the failing run)

questions for lead:
- Confirm the hand-over API (`Controller` / `ControllerHandle::take_rate_sessions`, `RateSession` re-exported from `lib.rs`) as the delivery for a derived kind, or say if you want a `tinwave-signals` change instead (a ticket outside this scope). Until a consumer exists, nothing reads the queue; an idle consumer costs 64 entries.
- Is the one-line `lib.rs` re-export acceptable? It was needed to name the type.
- Leftover processes: PIDs 73508 and 3760 (`nextest run -p tinwave-db`) and python 60496 (`cmake-build.cmd configure`, started 2-11) are not mine. I started nothing that is still running.
