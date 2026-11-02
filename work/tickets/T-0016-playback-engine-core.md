---
id: T-0016
title: tinwave-playback engine core from the T-0003 design (reader, ring, renderer, seek, position)
type: feature
epic: E-01
status: in_review
priority: P0
size: M
depends_on: []          # D-0003 decided; the engine emits PlaybackEvents, not signals
scope:
  - crates/tinwave-playback/
  - fixtures/audio/       # small self-made sweep fixtures (total <= 1 MB)
  - Cargo.toml            # [workspace.dependencies] only
  - Cargo.lock
  - deny.toml             # MPL-2.0 exception scoped to symphonia* (D-0003)
owner: tw-builder (round 2)
branch: t/T-0016-playback-engine-core
review_round: 2
created_by: from P-0022
---

## Context
Spike T-0003 (work/reports/T-0003/spike-1.md, prototype spikes/T-0003-audio-backend/src/engine.rs) and ADR 0005
(docs/adr/0005-audio-backend.md). Spec §7. docs/rules/architecture.md ("playback never blocks"), rust.md (Concurrency,
RT rules), testing.md. Signals need accurate position and volume hooks (signal-catalog §1, §3).

## Scope
- Backend-independent engine: decoder thread (Symphonia) -> lock-free SPSC ring (rtrb) -> RT renderer; gapless queue
  hand-off; seek handshake; atomic volume target with per-block ramp; 10 Hz position marks; device-reconfigure path.
- An `AudioOutput` trait the renderer is driven by; a test/null output used by the tests (real outputs are T-0017).
- Emits `PlaybackEvent`s that the signal layer turns into T0 signals (no signal construction inside the engine).

## Out of scope
OS outputs (T-0017), resampling/dither (T-0019), gapless metadata extraction (T-0018), time-stretch (T-0009).

## Acceptance criteria
- [ ] Generated split-sweep pair (WAV, FLAC, ALAC, MP3+LAME) plays across the boundary with a 0-frame gap — test
      recording the renderer output against the reference sweep.
- [ ] Seek lands sample-exact (max error <= 1.5e-5 lossless) — test.
- [ ] Zero heap allocations/frees inside the render callback across play, seek and volume changes — counting-allocator test.
- [ ] Position marks monotonic at 10 Hz — test.
- [ ] No decoder or file IO on the render thread — code review + test with a blocking reader.
- [ ] fmt, clippy `-D warnings`, tests, `cargo deny check` pass.

## Signals & storage
Produces the inputs for PlayStarted/Paused/Resumed/Seek*/Skipped/VolumeChanged; emission is wired in a later ticket.
