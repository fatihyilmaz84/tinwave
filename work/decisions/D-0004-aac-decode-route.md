---
id: D-0004
title: How should Tinwave decode AAC (M4A) — Rust software decoder or the OS decoder?
status: decided
blocks: []
date: 2026-11-02
---

## Question for the PO
AAC is still covered by an active patent pool (Via LA), which licenses decoders shipped in products. A pure-Rust
software AAC decoder (Symphonia) has no patent license attached. On Windows, the OS ships a licensed AAC decoder
(Media Foundation). Which route do we take?

## Context
- MP3 patents have expired; FLAC and ALAC are unencumbered. Only AAC (and HE-AAC) is affected.
- Symphonia decodes AAC-LC only (no HE-AAC); Media Foundation decodes AAC-LC and HE-AAC on Windows.
- macOS ships AudioToolbox (licensed AAC); Linux has no OS-level licensed decoder.
- This is not legal advice — a free, personal, open-source app may be judged differently from a commercial one
  (spec §16 Phase 9 "Commercial release" includes a license review).

## Options
1. **OS decoder where available** — Media Foundation on Windows, AudioToolbox on macOS; Symphonia AAC only on Linux
   (or AAC disabled there until reviewed). Licensed, adds HE-AAC on Windows/macOS; more platform code.
2. **Symphonia everywhere** — simplest, one code path; carries the patent exposure; no HE-AAC.
3. **Decide later** — use Symphonia for development now, revisit before any public release (Phase 9 review).

## Lead recommendation
Option 3 now (it doesn't block engine work: AAC sits behind the same decoder trait), with option 1 as the expected
outcome; the real-world corpus spike T-0020 measures Media Foundation as the AAC/HE-AAC route.

## PO decision
2026-11-02: **Option 3 — decide later.** Symphonia AAC in development behind the decoder trait;
revisit at the Phase 9 license review with the OS decoder as the expected route (ADR 0005 decision 2, T-0020).
