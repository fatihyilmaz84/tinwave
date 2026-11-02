REVIEW T-0031 (round 1)
verdict: APPROVE
checks (run by reviewer): fmt pass; clippy pass; tests 364 passed / 0 failed / 6 ignored; deny pass; stress 390 processes, 0 failures; default run opens no device (`--test device`: 1 passed, 4 ignored); ignored device tests not run.

Lead's checks:
1. Reverts (scratch copy of a16046f): reader fix reverted (`frame > length` back to `>=`) ⇒ 3 boundary tests fail (seek-to-end, last-track, switch-at-end); engine fix reverted (`cursor.is_none()` early return in `position()`) ⇒ 2 position tests fail; `seeking_past_the_end_is_still_a_failure` passes in both. New behaviour: `at_end` reader yields a mark at offset `length`, then EOF, then B's Continuation mark at the same `start_frame`; poll adopts A silently, then `TrackLeft{A, length, Finished}` + `TrackStarted{B, Advance}`; `at_end` can't leak (every seek opens a fresh TrackReader).
2. Seek to exactly the end vs catalog §3: consistent. `Completed` triggers on "reached end (or within last 3 %)"; seeks to `length − 1` or the last 3 % were already Finished; exactly `length` was the odd one out (SeekFailed ⇒ TrackLeft{Failed} ⇒ no signal). Now `Seeked` ⇒ SeekForward (to_ms = end) ⇒ Completed; `listened_ms` and SeekForward let taste discount it. For T-0024 (applies to every near-end seek): the T-0022 GestureCoalescer holds a seek gesture up to 2 s; a seek to the end is immediately followed by the track change, so T-0024 should `flush_all` (or the seek gesture) on TrackLeft/TrackStarted, or a pending seek on A could merge with a seek in B and land after A's Completed.
3. `position() = None` with no current track: no event T-0024 needs loses a position. `Seeked`: seek rejected when cursor and announced are None; with announced Some the early return gives the announced (item, frame). `TrackLeft` from load/stop use `current_or_announced()`; Finished/Failed leaves computed from marks in poll. `switch_device` no-current-track case uses its own unheard-mark path. Changed: Paused/Resumed/VolumeChanged after stop or queue end now carry None (catalog §1–3 need no position then). Subtle: between a command applied and poll adopting its first mark, `position()` returns the announced frame instead of the live one (stale by ≤ ~one poll interval + output latency, still monotonic).
4. `TINWAVE_PAIRING_SEEDS=256`: 5 pairing tests pass (89 s); rt_alloc mutation (Vec in Renderer::render) ⇒ output_rt_alloc fails (588); stress 13 binaries × 5 × 6 = 390, 0 failures.
5. Scope: reader.rs, engine.rs, event.rs (docs), output/windows.rs (doc), tests/; failures.rs past-end cases moved to length + 1 and 5_001.

acceptance criteria: all [x].

blocking findings:
- none

design check: n/a

non-blocking findings:
- none in the branch; the seek-coalescer flush on track change belongs to T-0024.

proposals: none

notes for lead: safe to merge. Carry to T-0024: flush the seek gesture on TrackLeft/TrackStarted; seek to the exact end ⇒ Seeked then TrackLeft{Finished} ⇒ SeekForward + Completed; event positions None when no track is current. Scratch: mut7/, stress-t31/.
