REVIEW T-0028 (round 1)
verdict: APPROVE
checks (run by reviewer): fmt pass; clippy `--workspace --all-targets -D warnings` pass; tests pass (`cargo test --workspace` 227 passed / 0 failed); deny pass; **stress: 540 concurrent processes, 0 failures**

Both T-0016 round-2 pairing bugs and the test hang are fixed; the property test catches both bugs when each fix is reverted. Two non-blocking gaps: the third bug is only caught at seed 38 (outside the default seed run), and appending after the queue ends still reports a stall as Underruns.

Lead's checks:
1. Round-2 repros (review_r2.rs vs 517b533): seek after QueueEnded ⇒ `Err(NothingLoaded)`, nothing emitted; failed switch ⇒ `[TrackStarted{1}, OutputLost, OutputReopened, TrackLeft{1,Finished}, TrackStarted{2,Advance}, TrackLeft{2}, QueueEnded]`; both pass `assert_paired`.
2. Stress: all 9 playback test binaries, 6 copies × 6 rounds = 324, then 4 × 6 = 216 ⇒ 0 failures in 540 (round 2: failures.rs hung once in 528; now uses `Harness::run_until`).
3. Property test vs pre-fix (scratch copy, each fix reverted separately): seek guard removed (engine.rs:270-272) ⇒ repro and property test fail (seed 1); `command_sent` back in the Ok branch only ⇒ repro and property test fail (seed 5); third bug (`Reconfigure{item: None}` resets `cur`) reverted, alone or with the old `view().ended` resume ⇒ default seeds 1..32 pass, only `TINWAVE_PAIRING_SEEDS=160` catches it (seed 38). Ticket only requires the two blocking bugs; non-blocking.
4. `seen_data` change acceptable: catalog has no Underrun/buffering signal; T-0024 states the contract; per-renderer state closes the device-switch window; shared `primed` handoff removed. One gap below (append after end).
5. Seek contract documented on `Engine::seek` (engine.rs:257-266); `Seeked` emitted only at engine.rs:280 after the `cursor`/`announced` guard and a successful send; in every `announced`-is-Some case a `TrackStarted` is still open, so `Seeked` can't occur without a current track.
6. rt_alloc: a Vec next to `seen_data |= …` fails with 784.
7. Scope: crates/tinwave-playback/ only (7 files).

acceptance criteria:
- [x] Two repros are tests, pass, and fail on pre-fix code — tests/pairing.rs + review_r2.rs.
- [x] Property test passes and catches both bugs on pre-fix code (seed 1, seed 5).
- [x] Concurrent stress ≥ 500 processes, 0 failures (540; builder 864).
- [x] Zero callback allocations; fmt, clippy, tests, deny.

blocking findings:
- none

design check: n/a

non-blocking findings:
- The third fix has no default-run regression test — decoder.rs:250-259 (keep `st.cur` on `Reconfigure{item: None}`) and engine.rs:375-377; reverting passes the default suite. Add a directed test: queue ended → append → `switch_device` before the appended track is heard; appended track plays, pairing holds.
- A stall after appending to an ended queue is still reported as Underruns — renderer.rs:89-91 with decoder `Cmd::Append` (`eos.store(false)` revival): Append clears `eos` while `seen_data` is still true from the previous stream, so every callback while the decoder opens the new file counts as an Underrun (probe: finish A, append B, 200 fast callbacks ⇒ 198 `Underrun`, 0 frames). Predates T-0028 (`primed` behaved the same) but contradicts the contract T-0024 relies on. Fix: decoder clears `eos` only with the first push after revival, or a stream-epoch atomic the renderer compares to reset `seen_data`.
- Narrow window where a just-appended track can be dropped — engine.rs:375-377: queue ended, a short appended track fully decoded into the ring but not yet heard (within output latency) ⇒ `cursor` and `announced` both None; a `switch_device` then resumes nothing and the decoder's `cur` is past the track ⇒ silently not played (no pairing violation). Needs a switch within ~1 latency period after an append.
- `Seeked` and `Underrun` docs lag the contract — event.rs:76 (`Seeked` only with a current track) and :121 (`Underrun` only after audio has flowed since the last open or flush). T-0024 will read these.

proposals:
- P: Run the pairing property test with more seeds in CI (e.g. `TINWAVE_PAIRING_SEEDS=256` nightly) | size: S

notes for lead:
- Safe to merge; main has no tinwave-playback changes since 219ed37.
- Pass the Append-after-end Underrun gap to T-0024 or a small follow-up.
- Scratch evidence (uncommitted): session scratchpad mut3/ (review_r2.rs, review_t28.rs), stress-t28/, stress-t28b/.
- Worktree not modified apart from target/.

---
Lead (2026-11-02): merged. Non-blocking 1-4 → T-0030 (P0, before T-0024). CI seeds proposal parked with P-0003.
