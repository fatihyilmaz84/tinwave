REVIEW T-0030 (round 1)
verdict: APPROVE
checks (run by reviewer): fmt pass; clippy `--workspace --all-targets -D warnings` pass; tests 231 passed / 0 failed; deny pass; **stress: 600 concurrent processes, 0 failures**

All three fixes confirmed by revert, the 256-seed property test passes; only a non-blocking one-callback race in `finish_queue` remains.

Lead's checks:
1. Reverts (scratch copy of a6e90ab): item 1 (`&& !buffering` removed, renderer.rs:93) fails the renderer unit test and append.rs:92; the T-0028 probe went from 198 Underrun callbacks to 0. Item 2 (decoder back to `cur = item…unwrap_or(len)`) fails `switch_while_an_appended_track_is_still_opening_does_not_drop_it` (via a ~16 s `wait_buffered` timeout). Item 3 (`unheard_start` disabled) fails `switch_after_an_appended_track_is_decoded_but_not_heard_does_not_drop_it`. Each test targets exactly its own fix. Item 4 docs-only; event.rs states both contracts.
2. Memory ordering: revival (decoder.rs:244-247 `buffering = true` then `eos = false`, Release) vs renderer loads `eos` then `buffering` (Acquire, renderer.rs:85-88): reading revived `eos = false` synchronises, so `buffering = true` or the later clear is visible; the clear (decoder.rs:195-199) is sequenced after the push; stale `eos = true` counts nothing. `!eos && !buffering` with an empty never-filled ring can't be observed across revival. Remaining window: `finish_queue` (decoder.rs:316-318 `buffering = false` then `eos = true`) for a revived queue that never pushes — one spurious underrun callback possible; swapping the order doesn't close it (renderer loads `eos` first); 1500 probes hit it 0 times.
3. Item 3 can't double-start: `TrackStarted{Advance}` only after a successful `send(Reconfigure)` (engine.rs:403-419) followed by `command_sent`; `reset_stream` clears old marks before the new Initial mark and `applied_gen`; `poll` returns early while unapplied, then adopts the new mark silently; failed open still recorded; branch only reached when `cursor` and `announced` are None after `poll()`. `TINWAVE_PAIRING_SEEDS=256` passes (84 s).
4. Stress: 10 playback binaries, 6 × 6 = 360 + 4 × 6 = 240 = 600 processes, 0 failures. rt_alloc mutation 762. Scope: crates/tinwave-playback/ only (8 files).

acceptance criteria:
- [x] Each item has a failing-before test (check 1; item 4 docs at event.rs:76-78, :126-130).
- [x] Property test and ≥ 500-process stress 0 failures; zero callback allocations.
- [x] fmt, clippy, tests, deny.

blocking findings:
- none

design check: n/a

non-blocking findings:
- decoder.rs:316-318 one-callback spurious Underrun in `finish_queue` (revived queue ending without a push). Fix: leave `buffering` set in `finish_queue` (`eos = true` already suppresses counting; next push / revival / `reset_stream` overwrite it).
- append.rs `switch_while_an_appended_track_is_still_opening_does_not_drop_it` fails by a slow (~16 s) timeout when its fix is reverted; a direct check (B never opened / no audio after the gate) would fail faster and say why.

proposals: none

notes for lead:
- Safe to merge. T-0024 contract holds: one TrackLeft per TrackStarted, no Seeked without a current track, buffering on revival is not an Underrun.
- Scratch evidence: session scratchpad mut4/ (review_t28.rs, review_t30.rs), stress-t30/, stress-t30b/.

---
Lead (2026-11-02): merged. Non-blocking 1–2 parked as P-0081 (fix when tinwave-playback is next touched by T-0017/T-0025).
