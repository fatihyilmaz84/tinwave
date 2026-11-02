REVIEW T-0017 (round 2)
verdict: APPROVE
checks (run by reviewer): fmt pass; clippy pass; tests 312 passed / 0 failed / 6 ignored; deny pass; stress 360 processes, 0 failures; ignored real-device tests not run.

Lead's checks:
1. OpenState (stream_thread.rs:33-55, 93-95; windows.rs:186-201): starts OPENING; only transitions are the thread's CAS OPENING→RUNNING and open's CAS OPENING→ABANDONED (AcqRel) ⇒ exactly one proceeds. `claim_run()` after the device is built, before `Pump::new` and the first `feed`; abandoned thread returns the renderer untouched. Failed open needs no claim (Err sent, open joins; send to a closed channel is harmless). Mutation (claim_run check disabled) fails both stream_thread tests (:233, :288). Post-claim wait: unbounded `recv()` (windows.rs:198-200), but every claimed path sends exactly once; a panic drops the sender ⇒ Disconnected handled and joined; bounded only by the driver returning from one feed + start_stream (same accepted hung-driver risk as Stream::drop; nothing better possible).
2. Engine fix (engine.rs:352-368, 2bd5e6a): while a track is current, `played` clamped to the start of the first unseen mark; unseen marks dropped before `resolve`; a not-yet-announced mark is pushed before any of its audio ⇒ clamp never lowers position; item changes only after poll announces it. Reverting fails `position_names_the_announced_track_until_poll_announces_the_next` ("left: (2, 0), right: (1, 48777)"). seek/switch_device call poll() first; the clamp matters only in µs between snapshots; seek acts on A (intended); switch_device resumes at (A, A_end) (non-blocking 1). `TINWAVE_PAIRING_SEEDS=256` passes (93 s).
3. rt_alloc: Pump mutation ⇒ 480; rt_alloc and output_rt_alloc pass.
4. Stress: 12 playback binaries × 5 × 6 = 360, 0 failures.
5. Default run opens no device (device test: 1 passed, 4 ignored); clip all-zero (device.rs:35-42).
6. MmcssGuard !Send (os.rs:22); realtime_priority from mmcss.is_ok(); drop-joins documented (windows.rs:91-97).

acceptance criteria: all [x] (gapless/seek through both outputs; re-open at same frame; exclusive integer + visible fallback; zero callback allocations; fmt/clippy/tests/deny; test safety).

blocking findings:
- none

design check: n/a

non-blocking findings:
1. A device switch exactly at a track boundary mislabels the finished track — engine.rs:370-372 with reader.rs (`frame >= length` ⇒ SeekOutOfRange): in the µs window `switch_device` resumes at (A, A_end = length); the reader rejects a seek to exactly `length` ⇒ SeekFailed, move to B ⇒ `TrackLeft{A, Failed}` + `TrackStarted{B, Advance}`. Pairing holds, but a completed track is labelled Failed (T-0024 would count no Completed). Very low likelihood. Fix: treat resume/seek to exactly `length` as finished-advance with no SeekFailed, or resume the next item when the clamped position equals the current track's end.
2. windows.rs:198-200 post-claim wait in `open` unbounded (driver-bound); mention next to the drop-joins note.
3. Pre-existing: with no current track (queue ended, appended track unheard), `position()` can name the unheard track; Paused/Resumed/VolumeChanged carry it ⇒ `position.item` with no TrackStarted. T-0024 should treat event positions as advisory then, or the engine returns None in that state.

proposals: none

notes for lead: safe to merge; item 1 fits a small engine follow-up or T-0024 grooming. Scratch: mut6/, stress-t17r2/.

---
Lead (2026-11-02): merged. Non-blocking 1–3 → T-0031 (engine boundary follow-up, before T-0024).
