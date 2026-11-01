---
id: D-0003
title: Adopt the T-0003 audio stack (Symphonia, MPL-2.0) and update spec §2/§7?
status: decided
blocks: [T-0016, T-0017, T-0019]
date: 2026-11-02
---

## Question for the PO
Spike T-0003 recommends **Symphonia 0.6 for decoding + our own playback engine + direct WASAPI output on Windows
(cpal as fallback and for macOS/Linux)**. Symphonia is **MPL-2.0** (file-level copyleft), which our `deny.toml` does
not allow today (MIT/Apache/BSD/ISC/Zlib/Unicode only). Accept the stack, allow MPL-2.0, and update spec §2 and §7?

## Context
- Measured on this PC: 0-frame gapless for WAV/FLAC/ALAC/MP3-with-LAME-tag, sample-exact seek, 0 allocations in the
  audio callback, 18 ms cold start (WASAPI), < 1 % CPU, exclusive mode works. Report: work/reports/T-0003/spike-1.md;
  draft ADR: spikes/T-0003-audio-backend/ADR-draft.md.
- MPL-2.0 obligations apply per file: if we ship Symphonia **unmodified**, we only provide its source/notice; our own
  code stays under our license. If we ever patch a Symphonia file, that file's changes must be published under MPL.
- Alternatives are worse on license or size: FFmpeg/libmpv (LGPL/GPL, large), BASS (commercial), rodio (rejected:
  decodes inside the audio callback).

## Options
1. **Accept** — allow `MPL-2.0` in deny.toml (exception scoped to the `symphonia*` crates), accept the ADR as 0005,
   update spec §2/§7, architecture.md and testing.md. Engine tickets T-0016/T-0017/T-0019 become ready.
2. **Accept, but no MPL anywhere** — needs a different decoder per format (claxon/minimp3/hound…), more code and gaps
   (no ALAC/AAC in pure MIT crates). Slower, more risk.
3. **Defer** — keep spiking (e.g. real-world corpus T-0020 first).

## Lead recommendation
Option 1, with the exception scoped to the `symphonia*` crates only and a rule that we never patch them in-tree
(upstream fixes or a wrapper instead).

## PO decision
2026-11-02: **Option 1 — accept**, MPL-2.0 exception scoped to `symphonia*`, never patched in-tree.
Applied: ADR 0005, spec §2/§7, architecture.md, testing.md. T-0016 ready; T-0017/T-0019 follow it.
