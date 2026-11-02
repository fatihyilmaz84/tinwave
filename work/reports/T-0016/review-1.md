REVIEW T-0016 (round 1)
verdict: CHANGES_REQUESTED
checks (run by reviewer): fmt pass; clippy `--workspace --all-targets -D warnings` pass; tests pass (`cargo test --workspace` 190 passed / 0 failed; tinwave-playback 25 runs debug + 15 runs release, all clean; **stress run: 4 failures in 336 test processes** when the test binaries run concurrently, see blocking 1); deny pass ("advisories ok, bans ok, licenses ok, sources ok")

acceptance criteria:
- [x] Split-sweep pair (WAV 16/24/f32, FLAC, ALAC, MP3+LAME) plays across the boundary with a 0-frame gap — verified by tests/gapless.rs. Lag 0 at head, tail and boundary; length pinned to exactly 96 000 frames; `TrackLeft` frames equal SPLIT; detector self-test sees ±1. Reference quantised like the writer, so 16-bit lossless is effectively bit-exact. MP3 0.02 justified (spike 1.06e-2; lag checks + length pin catch shifts).
- [x] Seek lands sample-exact — tests/seek.rs::seek_lands_sample_exact (1, 3, 12345, 20000, 40001, 47000, SPLIT-600 across the hand-off, two inside B; wav16/flac16/alac16/mp3). Pre-roll doesn't hide errors: `skip` recomputed from the container's `actual_ts` (reader.rs:172-177); lossless exact for targets below 4608 and near the end.
- [x] Zero heap allocations/frees in the render callback — tests/rt_alloc.rs; mutation check (`black_box(Vec::with_capacity(8))` in the flush branch) fails with "allocated or freed 8 times". `Renderer::render` (renderer.rs:51-118) touches only atomics, `&*self.sh` and the rtrb consumer; no lock, logging, format!, channel; no reachable panic path.
- [x] Position marks monotonic at 10 Hz — tests/position.rs.
- [x] No decoder/file IO on the render thread — code review + tests/blocking_reader.rs.
- [ ] fmt, clippy, tests, deny pass — pass in a normal run, but the suite is not deterministic under load (blocking 1).

blocking findings:
- crates/tinwave-playback/tests/common/mod.rs:242-251 (`Harness::settle`), also rt_alloc.rs:109-119, :253-264 (`finish`), position.rs:171-177 — **flaky acceptance tests, reproduced**: 4 failures in 336 concurrent processes (engine.rs:109 Paused position not 4800; position.rs:177 unexpected `Underrun` twice; common/mod.rs:263 "QueueEnded never arrived" in wav_float32_gapless). Cause: `settle` checks `command_pending()` then calls `render_block(256)`; the decoder can publish `applied_gen` and push new-stream audio in between, so `settle` consumes and discards real frames. `finish` waits only ~40 ms wall time for EOS; position.rs:171 doesn't filter legitimate `Underrun`. Fix: acknowledge the flush with `render(&mut [], latency)`; give `finish` the same 30 000-iteration bound; filter `Underrun` at position.rs:171 as engine.rs:186 does.
- crates/tinwave-playback/src/decoder.rs:251 (`begin_flush`) and :271 (`reset_stream`) — **corrupt-packet counts dropped silently**: both set `st.reader = None` without `take_corrupt_packets()`, so `CorruptPackets` is only emitted at natural track end (`finish_item`, :367-376). A glitch at 0:30 followed by a skip/seek/device switch at 0:45 never reports. A seek inside the track also resets the count. Breaks rust.md "Never swallow an error". No test produces `CorruptPackets`. Fix: one helper for every reader drop that reports the pending count first; a test that flips bytes in a copy of the MP3 fixture and asserts `CorruptPackets` at natural end and after skip/seek.

design check: n/a (no UI)

non-blocking findings:
- engine.rs:371-386 and renderer.rs:83: after `switch_device` the new renderer starts `primed == true`; starved callbacks before `Reconfigure` count as underruns, so every device switch can emit a spurious `Underrun` (engine.rs:183-189 calls them "expected"). Reset priming per renderer or before opening the new stream.
- reader.rs:177: `skip = stream_frame.saturating_sub(landed)` silently absorbs a container that lands after the target. Return `PlaybackError::Seek`.
- reader.rs:139-163: seek past the end only reported when the demuxer returns `OutOfRange`, only FLAC tested; a clamping demuxer advances with no `SeekFailed`.
- decoder.rs:353-362 with engine.rs:429-437: a track failing mid-decode is later reported `TrackLeft { cause: Finished }` alongside `TrackFailed`; a track failing to open after `load` gets `TrackStarted` but never `TrackLeft` (engine.rs:509 claims every TrackStarted gets a TrackLeft). The signal layer would count broken files as completed. Add a `Failed` leave cause or document the pairing.
- reader.rs:111-113, 186-224: `GaplessInfo` delay/valid-length trim path untested.
- No test for `TrackFailed(UnsupportedChannels)` (reader.rs:101, :214); the test WAV writer could emit 3 channels.
- tests/gapless.rs:51 and seek.rs:91: lossless tolerance 1.5e-5 allows ~125 LSB at 24-bit; the reference is exact, so `== 0.0` would be stricter.
- decoder.rs:130: after a failed `switch_device` the decoder polls for a flush ack at 1 ms (~1000 wakeups/s) until a retry.

lints/unsafe judgement: acceptable, not weakening (lib keeps `#![forbid(unsafe_code)]`, crate `unsafe_code = "deny"`, only tests/rt_alloc.rs opts in with a SAFETY comment). Cost: crate lint list can drift from `[workspace.lints]`. Narrower option in proposals.

licensing/architecture: verified — 15 per-crate MPL-2.0 exceptions matching the 15 `symphonia*` crates in Cargo.lock, no global MPL; no `lame` in Cargo.lock; all six fixture SHA-256 hashes match the README; fixtures 203 208 bytes; .gitattributes marks them binary; deps only tinwave-core, rtrb, symphonia, thiserror; no tinwave-signals; events stamped from the injected `Clock`; diff within scope.

concurrency review (no defects found): seek handshake sound (pushes happen-before `flush_req.fetch_add(AcqRel)`, renderer drains after Acquire, decoder pushes nothing until the ack); reconfigure relies on the documented drop contract (output.rs:31-33); Drop drops `tx`, idle decoder wakes as Disconnected; detached thread holds only Arcs.

proposals:
- P: Move the counting allocator into a tiny dev-only crate (or adopt `assert_no_alloc`, BSD-2-Clause) so tinwave-playback returns to `[lints] workspace = true` | size: S
- P: CI stress job running the playback test binaries in parallel under load | size: S

notes for lead:
- Normal-load runs clean (40 runs); flakes need concurrency, which is what CI runners are.
- Mutation check on a `git archive` copy in the session scratchpad (mut/); stress logs in stress1/, stress2/. Worktree not modified apart from target/.
- For T-0017: real outputs must honour the OutputStream drop contract (callback finished when drop returns) and must not drop the `Renderer` (holds the last `Arc<Shared>`) on the RT thread.
