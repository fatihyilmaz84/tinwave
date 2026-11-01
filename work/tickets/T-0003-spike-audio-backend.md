---
id: T-0003
title: "Spike: audio backend selection (decode + output + gapless)"
type: spike
epic: E-00
status: in_progress
priority: P0
size: M
depends_on: []
scope:
  - spikes/T-0003-audio-backend/
owner: tw-spike (round 1)
branch: t/T-0003-spike-audio-backend
review_round: 0
created_by: lead
---

## Context
Spec §7 and §2: choose the backend through a real-world test. Playback must never block (docs/rules/architecture.md).
Signals need accurate position and volume hooks (docs/guides/signal-catalog.md §1, §3).

## Question
Which Rust decode + output stack gives Tinwave gapless playback of FLAC, MP3, M4A/AAC, ALAC, WAV with reliable
seeking, device switching, and sample-accurate position on Windows?

## Scope
- Candidates at minimum: Symphonia + cpal; Symphonia + a WASAPI-specific output; one more if promising.
- Prototype: play a two-track gapless album sequence; seek; change default output device mid-playback; report
  position at 10 Hz; apply software volume without allocation in the callback.
- Measure: audio start latency, CPU while playing, RSS, gap at track boundaries (verify with a generated
  sine sweep split across two files), behaviour on device unplug.
- Check WASAPI exclusive mode feasibility (signal `ExclusiveModeToggled`).

## Time box
1 working session.

## Success criteria
Recommended stack with evidence, known gaps per format, draft ADR.

## Acceptance criteria
- [ ] SPIKE RESULT with format matrix (decode ok / seek ok / gapless ok) per candidate.
- [ ] Measured gap at boundaries, start latency, CPU, RSS.
- [ ] Licenses recorded; codec patent/licensing notes for AAC/MP3 if relevant.
- [ ] Draft ADR at `spikes/T-0003-audio-backend/ADR-draft.md`.

## Signals & storage
None, but confirm the position/volume hooks the signal pipeline needs are available.
