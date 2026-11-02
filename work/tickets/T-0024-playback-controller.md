---
id: T-0024
title: Playback controller — owns Engine::poll(), maps PlaybackEvents to T0 signals through the bus
type: feature
epic: E-01
status: backlog
priority: P0
size: M
depends_on: [T-0006, T-0030]   # T-0016 done
scope:
  - crates/tinwave-playback/src/controller/   # or a small crate named in grooming
owner:
branch:
review_round: 0
created_by: lead (T-0016 questions), P-0042
---

## Context
T-0016's core has no timer: the host must call `Engine::poll()` about every 100 ms (lead answer). PlaybackEvents
(TrackStarted, TrackLeft{Finished|Replaced|Stopped|Failed}, Paused, Resumed, Seeked, VolumeChanged, PositionMark, …)
become catalog signals (§1–§4: PlayStarted, Completed, Skipped, Seek*, Paused/Resumed, VolumeChanged…) through the
T-0005 bus, with in-app volume going through its own `GestureCoalescer`. architecture.md: playback emits facts,
signals are built outside the engine.

## Scope
- A controller thread owning poll() cadence and the event → signal mapping (with input source and position).
- A bounded `EventSink` with a reserved slot for playback-critical events (P-0042), per rust.md Concurrency.
- Emission tests for every mapped signal kind (testing.md "Signals must be tested").
- Implements `PlaybackContextSource` for tinwave-context's audio providers (P-0062) and loads/stores the
  `DeviceSecret` in the OS keychain (Windows Credential Manager first), generated via `&mut impl CryptoRng`.
- `PlaybackContextSource::current()` returns `Some` for the whole live session, **including while paused** (lead
  decision on the T-0014 review), so a headphone switch while paused still becomes `OutputDeviceChanged`.
- Engine contract after T-0028: `seek` with no current track is rejected and emits nothing; exactly one
  `TrackLeft` per `TrackStarted`; a stalled first read is buffering, not an Underrun.

## Acceptance criteria
- [ ] Each PlaybackEvent maps to exactly the catalog signals with the right payload, position and input source — tests.
- [ ] A failed track (`TrackLeft{Failed}`) produces neither `Completed` nor `Skipped` — test.
- [ ] poll() cadence from the injected clock in tests; no wall-clock reads in domain code.

## Signals & storage
Emits T0 playback signals; no new kinds.
