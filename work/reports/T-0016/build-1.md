HANDOFF T-0016 (round 1)
status: done
branch: t/T-0016-playback-engine-core   commit: 0a168bc
summary: New crate `tinwave-playback` with the T-0003 design rewritten to production rules. Symphonia decoder thread feeds an rtrb SPSC ring (1 s), and a real-time `Renderer` reads it behind an `AudioOutput` trait. `ManualOutput` is the deterministic in-memory output the tests use; real OS outputs stay with T-0017. The `Engine` control handle emits `PlaybackEvent`s through an `EventSink` and stamps them from the injected `Clock`. It does not touch tinwave-signals. deny.toml has the MPL-2.0 exception scoped per `symphonia*` crate. Fixtures are committed under fixtures/audio/ (about 200 KB).

acceptance criteria:
- [x] Split-sweep pair plays across the boundary with a 0-frame gap — `tests/gapless.rs` (7 tests: wav16, wav24, wav f32, flac16, alac16, mp3 with LAME tag, detector self-test). Lossless bit-exact (error 0.0); MP3 lag 0 both sides, max error 1.05e-2 (tol 0.02); length exactly 96 000 frames; `TrackLeft` frames equal the split; the self-test shows ±1 detection for a dropped/inserted frame.
- [x] Seek lands sample-exact — `tests/seek.rs::seek_lands_sample_exact` (12345, 1, 40001, 3, 47000, 20000, across the A|B hand-off, load into B and seek inside B; wav16, flac16, alac16, mp3). Lossless error 0.0; MP3 lag 0 within 0.02. Seek past the end gives `SeekFailed`, queue moves on.
- [x] Zero heap allocations/frees inside the render callback — `tests/rt_alloc.rs` (counting global allocator on the callback thread around each render call; play, volume ramps, pause/resume, seek handshake, device switch, track hand-off, end of stream, starved callbacks ⇒ 0). Fails with 800 when a Vec is added to `render`; `counter_detects_allocation` guards against a vacuous pass.
- [x] Position marks monotonic at 10 Hz — `tests/position.rs` (strictly increasing (item, frame), 4800 frames apart, 18–20 marks over 2 s, first at 4800; late poll yields one mark; marks follow heard audio, not the clock).
- [x] No decoder or file IO on the render thread — `tests/blocking_reader.rs` (a source blocking in `Read::read` before audio and mid-track: 2000 callbacks return silence at once, control calls return at once, watchdog; mid-track stall gives `Underrun`, not a stalled callback). `Renderer` touches only atomics and the ring.
- [x] fmt, clippy -D warnings, tests, cargo deny check pass.

checks: fmt pass, clippy pass, tests 124 passed / 0 failed (workspace; 31 in tinwave-playback: 2 rt_alloc, 12 engine, 7 gapless, 5 position, 2 blocking_reader, 3 seek). Playback tests run ~30× debug and 15× release for rt_alloc, no flakes. deny pass.
files changed: Cargo.toml (`rtrb`, `symphonia` with aac, alac, isomp4, mp3, aiff in [workspace.dependencies]), Cargo.lock (additions only), deny.toml (15 `[[licenses.exceptions]]` MPL-2.0 entries, one per symphonia* crate; no global MPL allow), crates/tinwave-playback (Cargo.toml, src/{lib,types,error,event,output,reader,renderer,shared,decoder,engine}.rs, tests/{common/mod,gapless,seek,rt_alloc,position,blocking_reader,engine}.rs), fixtures/audio (README.md + 6 binaries).
signals added/changed: none. PlaybackEvents: TrackStarted, TrackLeft (Finished/Replaced/Stopped), Paused, Resumed, Seeked, VolumeChanged, PositionMark, QueueEnded, TrackFailed, SeekFailed, CorruptPackets, Underrun, OutputLost, OutputReopened.
ui: none
docs updated: fixtures/audio/README.md (how made, CC0, SHA-256).
deviations from ticket:
- Fixtures: MP3+LAME, ALAC and FLAC16 pairs committed (generated once with the spike's gen.rs + driver, documented). WAV pairs written at test time by a 30-line writer in tests/common. No flacenc/hound/mp3lame in the workspace; Cargo.lock has 0 lame entries.
- Crate `[lints]` not `workspace = true`: the counting-allocator test needs `unsafe impl GlobalAlloc` and a workspace `forbid` cannot be locally allowed. Crate sets `unsafe_code = "deny"`, library root keeps `#![forbid(unsafe_code)]`, only tests/rt_alloc.rs allows unsafe; other workspace lints replicated.
- Seek pre-roll (fix to the spike design): accurate seek on the target packet corrupted ~500 MP3 frames (error up to 0.05); reader seeks 4608 frames early and decodes-and-drops to the target.
- Lossless error checked against a reference quantised like the generator (`round(x*32767)/32768`) ⇒ exactly 0.0 (spike's 1.83e-5 came from the 32767 vs 32768 scale).
- Pause/resume ramp across one block (click-free), beyond the spike; position stays exact.

proposals:
- P: Live "set upcoming" (play-next, reorder) without a flush | size: M | evidence: engine.rs, `Engine::append` is the only live queue edit
- P: Bounded `EventSink` implementation with a reserved slot for T0 events | size: S | evidence: src/event.rs, rust.md Concurrency
- P: Short fade on seek and track-skip flush (clicks) | size: S | evidence: src/renderer.rs flush branch
- P: CI job for fixtures/audio provenance (hash check against the README) | size: S
- P: Audit downmix/resample policy (>2 ch ⇒ TrackFailed(UnsupportedChannels), rate mismatch ⇒ TrackFailed(SampleRateMismatch)) when T-0019 lands | size: S

questions for lead:
- Who calls `Engine::poll()` ~every 100 ms (the core has no timer)?
- `Drop` for `Engine` signals without joining; `shutdown(self)` joins. Confirm.

---
Lead answers (2026-11-02): poll() is owned by the playback controller that wires PlaybackEvents to signals (later
E-01 ticket), not T-0017; document it on `Engine`. Drop-without-join / shutdown-joins confirmed; document both.
Branch must merge main (T-0005 landed first) before review.
