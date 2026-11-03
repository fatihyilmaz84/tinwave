HANDOFF T-0068 (round 2)
status: done
branch: t/T-0068-queue-edit-cancels-boundary-park   commit: 19fd4c6
summary: The blocking finding is fixed. `heard_floor` is now an `AtomicU64` that every `view()` read during the drain raises to the position shown. A queue edit that withdraws the park therefore freezes the position instead of dropping it, and this also covers `position()` reads between polls. I added the reviewer's repro as an integration test, in a same-rate, an other-rate and a remove variant. The same-rate test fails on the old behaviour with exactly the reviewer's numbers.

acceptance criteria:
- [x] [a 44.1k, b 96k] parked for b, play-next c 44.1k: no 96 kHz open, a→c gapless and bit-exact — proof: `exclusive_reopen::a_play_next_during_a_boundary_park_keeps_a_same_rate_hand_off_gapless_and_opens_no_stream` (unchanged, passes).
- [x] Edit to another rate re-parks for that rate — proof: `an_edit_to_a_track_of_another_rate_parks_again_for_that_rate` (unchanged, passes).
- [x] P-0294: the position follows what is heard and never goes back — proof: `the_position_follows_the_device_buffer_while_the_old_stream_plays_out_and_never_goes_back`, plus the three new tests below.
- [x] Stable under parallel stress; fmt, clippy, nextest, deny pass — proof: `cargo nextest run -p tinwave-playback --stress-count 10` gave 10/10 passed. Workspace nextest: 838 passed, 7 skipped.

round 2 changes:
- `crates/tinwave-playback/src/engine.rs`:
  - `heard_floor: AtomicU64`.
  - `view()` does `fetch_max(shown)` whenever the drain is credited. `service_reopen` raises it with `fetch_max(total)` after the claim. Atomic because `position()` takes `&self`.
  - Field and `view` comments updated.
- `crates/tinwave-playback/src/decoder.rs`: a doc comment on `reconsider_park` for the refused-rate resampler (see below).
- `crates/tinwave-playback/tests/exclusive_reopen.rs`: helper `position_across_an_edit_in_the_drain` and three tests:
  - `a_same_rate_edit_in_the_drain_does_not_put_the_position_back`
  - `an_edit_to_another_rate_in_the_drain_does_not_put_the_position_back`
  - `removing_the_tail_in_the_drain_does_not_put_the_position_back`
  - Each runs `wait_parked`, renders a fully, then two `render(100)` calls with a poll after each. It then takes `before = position()` and asserts it is above the uncredited 23478, applies the edit, and takes `after`. It asserts `(before.item, before.frame) <= (after.item, after.frame)`, and that the position stays monotonic over four more callbacks.
  - The helper allows `position()` to be `None` only in the remove variant, where the queue has ended.

failing-on-old proof (mutation: the `fetch_max(shown)` in `view()` replaced by `let _ = shown;`, which is the old behaviour; the file was restored and touched):
`FAIL a_same_rate_edit_in_the_drain_does_not_put_the_position_back — the position went back: Position { item: 1, frame: 23695 } then Position { item: 1, frame: 23478 }`. That is the reviewer's numbers. Restored: 22/22 in `exclusive_reopen`.

Notes on the other two variants:
- Remove: the position cannot go back there, because the queue ends and `ended` snaps it to `consumed`, so it goes forward. The test documents that behaviour and covers the edit path, but it does not fail on the mutant.
- Other rate: its outcome under the mutant depends on how fast the decoder re-parks, which restores the credit. With the fix it is deterministic.

checks: fmt pass, clippy pass (`--workspace --all-targets -D warnings`), tests 838 passed / 0 failed (7 skipped), deny pass, `cargo build --workspace` pass
files changed: `crates/tinwave-playback/src/engine.rs`, `crates/tinwave-playback/src/decoder.rs`, `crates/tinwave-playback/tests/exclusive_reopen.rs` (round 1 also changed `crates/tinwave-playback/src/shared.rs`)
signals added/changed: none (round 1 position marks during the drain tail: same kind, tier and rate)
ui: none
docs updated: rustdoc only
deviations from ticket: none (the `shared.rs` addition was accepted by the lead)

non-blocking findings:
- Lost-claim coverage at `engine.rs` `service_reopen`: left for backlog T-0081, as agreed.
- Refused-rate resampler after a withdrawal: I agree with the reviewer and added a code comment on `reconsider_park`. A successor at the head's file rate gets a fresh resampler (a flush plus a new priming, same frame count, not bit-exact anyway).

proposals:
- P: Hold the engine's claim back while a `replace_after` is unanswered | why: avoids the one wasted open and gap when an edit loses the race | size: S | evidence: `engine.rs` `service_reopen`, `decoder.rs` `reconsider_park`
- P: Integration test for the claim-first interleaving (T-0081) | why: only a unit test covers it | size: S | evidence: `decoder.rs` unit test `a_queue_edit_after_the_engine_took_the_request_leaves_the_decoder_parked_for_the_resume`

questions for lead: none. Nothing I started is still running; the `cargo.exe` visible in tasklist is not mine.
