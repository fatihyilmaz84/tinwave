REVIEW T-0027 (round 1)
verdict: APPROVE
checks (run by reviewer): fmt pass, clippy pass (`--workspace --all-targets -D warnings`), tests pass (branch dfa60b9: nextest 675 passed / 6 skipped, exit 0; merged tree main+branch (`git merge-tree` a51f295, clean, no conflicts) built in a git-archive scratch with its own CARGO_TARGET_DIR: fmt/clippy pass, nextest 697 passed / 6 skipped, deny ok), deny pass (advisories, bans, licenses, sources ok on both)

acceptance criteria:
- [x] Zero callback allocations across rate changes — verified by: `tests/rt_alloc.rs::render_never_allocates_across_rate_changes`. It covers 5 rate steps, a stretched seek, a change while paused, a device switch and the end of the track. It passed 24/24 alongside 16 copies of rate.rs, then 8/8 alongside 32 copies of rate.rs on 16 logical CPUs. Code reading of `renderer.rs` `flush`/`fade_in`/`shape`/`account`: atomics, rtrb pop/read_chunk and arithmetic only. No lock, no Vec, no blocking.
- [x] Position accuracy ≤ 3 ms mean through 4 rate changes — verified by: `rate::position_accuracy_through_four_rate_changes_is_within_3_ms_on_a_click_train` (48 kHz) and `position_accuracy_holds_when_the_file_has_another_rate_than_the_stream` (44.1 kHz, stretch then resample). Both pass under load.
- [x] Time-weighted rate within 0.1 % — verified by: `rate::time_weighted_rate_matches_the_schedule_within_a_tenth_of_a_percent`. On its own this is close to a tautology, because the empty-callback acks make `src_micro` exact. The test is still meaningful: it also checks marks-based `position()` against `src_micro` within 10 frames, and those are two independent books.
- [x] No click at a rate step — verified by: `rate::no_click_at_a_rate_step_…` (bound 1.03× an analytic sine) plus my own probe (a scratch copy of rate.rs with an extra test, 3 runs). The largest step within ±1500 frames of each restart was 0.02890 (1.0035× analytic). The largest step elsewhere, in steady stretched audio, was 0.02895 (1.0053×). So the literal criterion holds: the step at a restart is no larger than the steady stretched signal's. The builder's "stretcher ripple" argument is sound, not a cover for a problem.
- [x] Bypass at 1.0 is bit-exact — verified by: `rate::bypass_at_one_is_bit_exact_after_a_trip_through_the_stretcher`. Every sample after the 240-frame fade-in is bit-equal, and the fade-in is an exact ramp. In code, `stage_for` returns `None` at 1.0 and `Mark::source_frame_at` stays integer at 1.0.
- [x] P-0206 BlockPlan / P-0209 two-sided onset — verified by: `stretch/plan.rs` (unit tests and doctest), `tests/stretch.rs::drive` uses `BlockPlan`, and `the_onset_lands_at_the_source_position_over_and_under_discarding_both_fail` asserts ±256-frame lag errors on their own sides.

blocking findings:
- none

design check: n/a (no UI)

Specific questions from the lead:
- **`stream_rate` vs the ring:** no window found. The ordering is:
  - `stream_rate` is written only by `adopt_rate`.
  - It is called from `reset_stream`, after the decoder observed `flush_ack` (Acquire) and the renderer had drained the ring, or on a fresh ring after the old renderer was dropped (Reconfigure).
  - It is also called from `service_rate` when `!audible()`. That means no marks, or `eos && consumed >= eos_total`, so the ring is empty, and that path is skipped while a flush is awaited.
  - The decoder's Release store comes before its rtrb push (Release on the write index). The renderer's pop (Acquire) comes before its `stream_rate()` load (Acquire). So popped frames are always booked at their own rate.
  - Fade-out frames are booked before the ack, and the decoder does not adopt until after the ack, so they are booked at the old rate.
- **Five deviations:**
  - No segment queue: sound, since the ring only ever holds one rate.
  - `Mark.rate`: consistent with the existing mark machinery. Retaining marks with `start <= consumed` never drops a mark the control side has seen, because it only consumes marks with `start <= played <= consumed`. The next-track case is tested at 4,800 frames of latency, with a single TrackLeft/TrackStarted pair.
  - Pre-roll: measured, and `seek_with_preroll` handles start, end and exact-end correctly.
  - Multi-callback fade-out: correct, including a seek arriving mid-fade (the fade completes, no fade-in, and the latest req is acked).
  - 125 ms throttle: the latest target is re-read at restart, waits are capped through `wait_for_command`, and there is no busy loop (a non-zero wait is returned; zero means flush now).
- **Signals:** RateChanged goes through the 600 ms coalescer, needs a session (or an unheard play, which opens one), is not built when Speed is off (tested), and carries the input source. Restarts emit nothing (same-item Rate mark → `{}`).
- **Test timing:** the 130 ms sleeps are only a lower bound for the decoder's `Instant` throttle, so load only makes them safer. The drag test's lower bound of 2 flushes could only fail if the decoder thread were descheduled for more than about 470 ms. No failures in 64 parallel copies.

non-blocking findings:
- crates/tinwave-playback/tests/rate.rs:359 — The bound is 1.03× an analytic sine. My probe shows 1.0035× near restarts against 1.0053× steady, so a relative check (max step within about ±30 ms of each restart ≤ max step elsewhere in the same recording) would match the criterion literally. It would also catch a fade of about 2 ms (≈1.016×), which the 3 % bound lets through. Fade lengths are pinned elsewhere, so this is not blocking.
- crates/tinwave-playback/src/engine.rs:465-470 (`set_rate` rustdoc) — It claims "about 15 ms of dip and 10 to 20 ms from the call to the first audio". Those are the spike's numbers, taken before pre-roll and with no handshake backoff. Here the decoder's flush wait backs off 1→2→4→8→16 ms (decoder.rs:233-242), so on a real device it can notice the ack up to about 2× the ack time late, and the silent gap is longer. Nothing in this ticket measures it. Reword the rustdoc, or keep it out of ADR 0006/spec docs, until the listening/latency pass the builder proposed.
- crates/tinwave-playback/src/decoder.rs:530-557 (`restart_audible`) — A restart re-reads the track. If it fails mid-decode again, or the file has vanished, `TrackFailed` / corrupt-packet reports are emitted again for the same item. If the reopen fails, the eventual TrackLeft says Finished rather than Failed. Rare edge cases.
- crates/tinwave-playback/src/decoder.rs:493 — The throttle uses std `Instant`, not an injected clock. That is acceptable (timing, not domain logic), but an injectable throttle clock would make rate.rs about 1 s faster and fully deterministic. The builder already raised this.

proposals:
- P: Relative no-click assertion | why: compare the max step near restarts with the same recording's steady max instead of 1.03× analytic; catches fades ≥ 1.5 ms short | size: S
- P: Skip the backoff for rate flushes | why: reset `flush_wait` to the minimum (or poll at 1 ms) while awaiting a `Restart::Audible`, so the dip is not stretched by up to about 15 ms of backoff | size: S
- P: Gapless boundary listening check while stretched | why: each track gets `finish()` (zero-padded tail) and a cold `begin()`. Cold start overshoots up to about 4–5 % for about 60 ms (measured by the builder), so continuous gapless albums played at ≠1.0 may show a small blur or level bump at the junction. Add it to the PO listening pass | size: S

notes for lead:
- Merge is clean against current main (T-0029/T-0058 don't touch tinwave-playback). The merged tree passes all gates (697 tests).
- For the docs (P-0248), points the code supports or doesn't:
  - RateStats counters are global and booked at the callback, not at the speaker. A per-play RateSession (T-0062) taken from snapshots at PlayStarted/TrackLeft will be off by about one device latency at the boundaries, so docs should not call it exact per play.
  - ADR decision 5's "audible position = base + src_micro_since_flush" is replaced by `Mark.rate`.
  - ADR "largest step equals the steady sine's" is better phrased as "≤ the steady stretched signal's", which is true per my probe.
  - Don't carry the "10–20 ms" latency into spec §7 as a measured figure.
- Housekeeping:
  - A `cargo nextest run --workspace --no-fail-fast` (PIDs 67008/50588, started 14:58) is running and is not mine, probably the T-0060 builder's. I left it.
  - `%TEMP%\tinwave-playback-rate-idle-deferred-65488` (14:39) may predate my runs. I did not delete it since I can't confirm it's mine.
  - My scratch dir (merged tree, its target dir, probe and stress outputs) has been deleted, and the worktree is untouched (`git status` clean).

