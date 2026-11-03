HANDOFF T-0068 (round 1)
status: done
branch: t/T-0068-queue-edit-cancels-boundary-park   commit: 5134056
summary: A queue edit during an exclusive boundary park now withdraws the park. The decoder carries on, so a successor at the stream's rate continues gapless in the same ring and one of another rate parks again for its own rate. The engine and the decoder race for the reopen request with a compare-exchange, so an edit that loses that race leaves the decoder parked for the `Resume`. P-0294 is done too: the position and the 10 Hz marks follow the device buffer during the drain and never go back when the new stream's latency starts counting. One scope deviation: I also touched `crates/tinwave-playback/src/shared.rs` (see below).

acceptance criteria:
- [x] [a 44.1k, b 96k] parked for b, play-next c 44.1k: no 96 kHz open, a→c gapless (bit-exact, no silence) — proof: `exclusive_reopen::a_play_next_during_a_boundary_park_keeps_a_same_rate_hand_off_gapless_and_opens_no_stream`. It uses `render_exact`, which fails on any starved callback, and compares a++c against the files bit for bit. Opens are `[48000, 44100]` and `underruns==0`.
- [x] Edit to another rate re-parks for that rate — proof: `an_edit_to_a_track_of_another_rate_parks_again_for_that_rate` (88.2 kHz: opens `[48000, 44100, 88200]`, a++d bit-exact, old stream played out first). Also `removing_what_follows_during_a_boundary_park_ends_the_queue_after_the_current_track`. The Resume-for-a-withdrawn-park case is covered by the compare-exchange protocol, not by ignoring a stale Resume; see the deviation below.
- [x] P-0294: the position follows what is heard during the drain — proof: `the_position_follows_the_device_buffer_while_the_old_stream_plays_out_and_never_goes_back`. It checks that the position is monotonic across the drain, the reopen and the first frames of the new stream, that it ramps through the drain and ends exactly at the end of file 1.
- [x] Stable under parallel stress; fmt, clippy, nextest, deny pass — proof: `cargo nextest run --workspace --stress-count 12` gave 12/12 iterations passed (835 tests each). Single runs: 835 passed, 7 skipped.

checks: fmt pass, clippy pass (`--workspace --all-targets -D warnings`), tests 835 passed / 0 failed (12 new: 5 decoder, 3 engine/shared unit, 4 integration), deny pass (advisories, bans, licenses, sources ok)

mutations tried (every new test fails when the code it covers is undone; each file was restored with a fresh mtime):
- Dropping `reconsider_park` in `edit_queue` failed 2 decoder unit tests and 3 integration tests (play-next, other rate, remove).
- Dropping the engine's `take_reopen` failed `the_engine_takes_the_request_it_answers...`.
- Setting the drain credit to 0 failed the position test.
- Dropping `.max(heard_floor)` failed the position test ("position went back").

files changed:
- `crates/tinwave-playback/src/decoder.rs`: `Parked.target`, `reconsider_park` called from `edit_queue`, module docs, 5 unit tests.
- `crates/tinwave-playback/src/engine.rs`: `service_reopen` takes the request; `heard_floor`; `View.shown` and `announced()` shared by `position()` and the 10 Hz marks; docs; 2 unit tests.
- `crates/tinwave-playback/src/shared.rs`: new `Shared::take_reopen(id, rate) -> bool` (compare-exchange of the request slot) and a unit test.
- `crates/tinwave-playback/tests/exclusive_reopen.rs`: 4 integration tests.

signals added/changed: none. Position marks are emitted during the drain tail, but the kind, tier and volume are unchanged: still about 10 per second of heard audio, at most about 5 more per boundary reopen. This is not a new signal.
ui: none
docs updated: rustdoc only (decoder module header, `Engine::replace_after`, `Engine::position`, `service_reopen`). No guide or catalog change needed.

deviations from ticket:
- I edited `crates/tinwave-playback/src/shared.rs`, which is outside the ticket scope. I could not make the withdrawal race-free in scope: `Shared::reopen` is stored through a private `pack_reopen`. Without an atomic claim, an edit that withdraws the park while the engine is already opening the stream leaves a `Resume` with nobody to take it, and the new stream then gets no audio. I added only `take_reopen` (about 15 lines) and its test; `unpark` and `park` are unchanged. Please say if you want this handled differently. It does not touch `controller/`, so it should not collide with T-0070.
- The ticket says a Resume for a withdrawn park should be "ignored safely". With the compare-exchange it cannot arrive: the engine takes the request before closing the old stream, and the decoder withdraws only if the request is still there. If the engine took it first, the decoder stays parked, applies the edit to the queue, and on the `Resume` looks at the edited successor, re-parking if its rate differs. That costs one wasted open in the rare edit-lands-after-claim race, which is unavoidable once the old stream is closed. The existing "not pending" branch in `resume()` stays as a guard, with its comment updated.

proposals:
- P: Hold the engine's claim back while a `replace_after` is still unanswered | why: an edit queued but not yet taken by the decoder at the moment the old stream has played out loses the race and costs one wasted open at the old successor's rate; correct but audible as a gap | size: S | evidence: `engine.rs` `service_reopen` (the take) and `decoder.rs` `reconsider_park` (the CAS-loses branch, covered by the unit test `a_queue_edit_after_the_engine_took_the_request_leaves_the_decoder_parked_for_the_resume`). It needs an edit-acknowledged counter in `Shared`.
- P: Integration test for the claim-first interleaving (edit sent, engine claims before the decoder processes it) | why: only unit-level today, because the interleaving is not deterministic through the public API | size: S | evidence: the CAS-loses branch of `reconsider_park`.

questions for lead:
- Is the `shared.rs` addition acceptable as part of T-0068's scope? If not, the alternative is to duplicate the `(id<<32)|rate` packing in `engine.rs` and `decoder.rs`, which I would not recommend.
- One full-workspace run failed once under parallel load, on `an_edit_to_a_track_of_another_rate_parks_again_for_that_rate`. The test asserted `reopen_pending()` right after `QueueEdited`, but the decoder re-parks a moment after reporting the edit. I fixed it by waiting for the new request (`wait_parked`), and the 12 stress iterations afterwards were clean. I did not see the failure message, so that cause is inferred from the code.
- Nothing I started is still running. A `python.exe` (PID 60496) is on the machine; it is not mine, since I only ran `python -c "print(1)"` once and it exited. I could not kill my own stress run (the permission classifier denied `Stop-Process`), so I let it finish; it has completed.
