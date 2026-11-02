---
id: T-0024
title: Playback controller — owns Engine::poll(), session lifecycle, maps PlaybackEvents to T0 signals through the bus
type: feature
epic: E-01
status: in_review
priority: P0
size: M
depends_on: []          # T-0006, T-0016, T-0028, T-0030, T-0031 done
scope:
  - crates/tinwave-playback/src/controller/   # new module
  - crates/tinwave-playback/src/lib.rs        # module registration / re-exports
  - crates/tinwave-playback/src/event.rs      # StreamInfo in OutputReopened (P-0082)
  - crates/tinwave-playback/src/engine.rs     # only to carry StreamInfo into OutputReopened
  - crates/tinwave-playback/Cargo.toml        # tinwave-signals dependency (allowed: playback ─> signals)
  - crates/tinwave-playback/tests/
  - Cargo.lock
owner: tw-builder (round 1)
branch: t/T-0024-playback-controller
review_round: 1
created_by: lead (T-0016 questions), P-0042, P-0082; regroomed 2026-11-02
---

## Context
architecture.md: `tinwave-playback ─> tinwave-signals (emits)`; the engine emits facts (PlaybackEvents), the
controller turns them into catalog signals; `tinwave-playback` must NOT depend on `tinwave-context` (the
`PlaybackContextSource` adapter and the keychain live in the desktop composition root / tinwave-context — T-0033 and
the E-01 shell). Engine contract (event.rs docs, T-0028/T-0030/T-0031): exactly one `TrackLeft` per `TrackStarted`;
`seek` with no current track is rejected and silent; event positions are None when no track is current; a seek to
exactly the end is `Seeked` then `TrackLeft{Finished}`; a stalled first read is buffering, not an Underrun; the host
calls `Engine::poll()` about every 100 ms. Catalog §1–§4, §8; testing.md "Signals must be tested".

## Scope
- `Controller` owning the `Engine`, a `SignalBus` handle and the poll cadence (a thread, timed by the injected `Clock`
  in tests). Commands carry `InputSource` and origin (`PlayOrigin`, `QueueOrigin`) so signals get them.
- Map every PlaybackEvent to catalog signals: PlayStarted (origin, input), Completed (listened_ms, paused_ms; "reached
  end or within last 3 %"), Skipped (pos_ds, fraction, input, section, queue_origin), SeekForward/SeekBack (> 2 s
  summed jumps via the T-0022 coalescer rules), Paused/Resumed (pause_ms, input), VolumeChanged (in-app gesture,
  600 ms coalescer), RateChanged when rate control exists (not yet — leave a clear hook, no placeholder code),
  PreviousPressed, RepeatModeChanged, Queue*/Favorited if the command surface includes them (otherwise list as
  follow-ups). `TrackLeft{Failed}` produces neither Completed nor Skipped.
- Section-related fields (`section`, `ms_since_section_start`) are `Unknown` until audio analysis exists — say so in docs.
- Own `GestureCoalescer`s (volume 600 ms, seek 2 s) and `flush_all` them on TrackLeft/TrackStarted (T-0031 review).
- Listening sessions (catalog §8): `SessionStarted` on the first play after ≥ 30 min of silence, `SessionEnded` after
  30 min of silence (timed by the injected clock); `SessionId` from `IdGenerator`; call `bus.set_tz_offset` at start.
- Expose a small read API for the shell: current session id, current recording, playing/paused (the desktop wiring
  will adapt it to tinwave-context's `PlaybackContextSource`; a paused live session counts as live).
- `StreamInfo` (mode, format, latency, exclusive fallback reason) in `OutputReopened` (P-0082).
- A bounded `EventSink` between engine and controller with a reserved slot for playback-critical events (P-0042,
  rust.md Concurrency); position marks may be dropped/coalesced, TrackStarted/TrackLeft never.

## Out of scope
OS keychain / DeviceSecret (T-0033); PlaybackContextSource adapter and app wiring (E-01 shell); time-stretch (T-0027);
live queue edits (T-0023).

## Acceptance criteria
- [ ] Emission test per mapped signal kind: exactly one event with the right payload, position and input source
      (engine on a `ManualOutput`/fake output, real bus, `ManualClock`).
- [ ] `TrackLeft{Failed}` ⇒ neither Completed nor Skipped; seek-to-end ⇒ SeekForward + Completed; seek gesture never
      merges across a track change — tests.
- [ ] Session start/end at the 30-minute boundaries (29:59 vs 30:00) — tests with `ManualClock`.
- [ ] Disabled category ⇒ zero signals constructed for those kinds (bus toggles) — test.
- [ ] Engine critical events never dropped by the bounded sink under a flood of position marks — test.
- [ ] Zero callback allocations still hold (rt_alloc); pairing property test green; no wall-clock reads in domain code.
- [ ] fmt, clippy `-D warnings`, tests, `cargo deny check` pass; default run opens no device.

## Signals & storage
Emits T0 playback and session signals; no new kinds.
