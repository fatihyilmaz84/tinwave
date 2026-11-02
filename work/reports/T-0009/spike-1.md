SPIKE RESULT T-0009
answer: Use Signalsmith Stretch (MIT) run in the T-0003 decoder thread, bypassed at exactly 1.0x, with each rate change done as a flush-and-restart at the audible source position (5 ms fades). Wrap the C++ headers ourselves with cc/cxx, because the `signalsmith-stretch` crate needs libclang at build time.
confidence: medium-high on CPU, memory, latency, RT-safety and position accounting (all measured). Medium on audio quality, because it is judged only by objective measures on synthetic material and nobody has listened.

evidence:
- Machine: Windows 11 Pro 10.0.26200, Ryzen 7 7800X3D, Rust 1.92 release, MSVC 14.50. Material generated in `src/synth.rs` (120 BPM drums from noise bursts, 4-chord detuned-saw pad, 0.5 s sine melody, 44.1 kHz stereo). No music downloaded.
- CPU on one pinned core, 60 s of audio, 1024-frame blocks, best of 5, % of one core in realtime (0.75x / 1.25x / 1.5x), results/bench.txt:
  - Signalsmith default 1.1 / 1.4 / 1.5 %; cheaper 1.0 / 0.9 / 0.9 %; block 2048/interval 512 1.1 / 1.2 / 1.2 %
  - Rubber Band R3 finer 8.1 / 9.6 / 11.6 %; R2 faster 2.7 / 2.6 / 3.1 %; WSOLA 15 ms hop 5.9 / 5.9 / 6.0 %
  - 0 heap allocations in steady-state processing for every option (Rust allocations + C++ `operator new` hook).
  - Rubber Band built from its single-file source with the built-in FFT (likely pessimistic).
- Memory per instance (private-bytes delta over 8 instances), results/mem.txt: Signalsmith 0.9 / 0.8 / 0.3 MB; Rubber Band R3 6.8 MB, R2 8.1 MB; WSOLA 1.0 MB. Object code: Signalsmith 0.7 MB, Rubber Band 3.8 MB.
- Latency (3 ms click), results/latency.txt: Signalsmith default 120 ms at 1.0x, 140 ms at 0.75x, 100 ms at 1.5x (2646 + 2646/rate frames at 44.1 kHz); cheaper 140 ms; b2048 46 ms; Rubber Band ~0 at 1.0x (−17..+16 ms); WSOLA 7–13 ms.
- Artifacts on the synthetic mix (results/quality.txt):
  - Pitch error median/max cents: Signalsmith 0.1 / 1.6, WSOLA 0.1 / 0.6, R3 0.1 / 9.6, R2 4–8 / 29 (floor 0.05).
  - Hat rise time at 1.25x (source 2.0 ms): WSOLA 3.0, R3 3.9, b2048 4.8, Signalsmith default 5.8, R2 15.1. At 0.75x: Signalsmith default 13.8 ms (dull hats), R3 5.6, WSOLA 2.1.
  - Pre-echo: Signalsmith −32 to −38 dB, R3 −41 to −44 dB, WSOLA none.
  - Pad log-spectral distance: Signalsmith default 6.7–7.7 dB (best), WSOLA 7.5–8.2, R3 8.6–8.9, b2048 10–12.
  - 1 kHz sine purity ~41.6 dB Signalsmith default and WSOLA, 39–41 dB R3; R2 collapses (untuned).
  - Stereo correlation preserved; Signalsmith level change −0.07 to −0.19 dB.
  - WAVs for a listening pass: `t0009-time-stretch-spike quality wav` (~100 MB in out/, git-ignored).
- Integration sim (src/engine_sim.rs; simulated 10 ms callback), results/engine.txt:
  - 0 allocations/frees in `Renderer::render` over 900 callbacks and 5 rate changes; max callback 16–44 µs; 0 underruns.
  - Rate-change latency with flush-and-restart 10–19 ms (median 13–17; one 41 ms outlier); without flush = ring fill (160 ms at 150 ms target … 994 ms at 1 s).
  - Rate-change artifact ~15 ms below 70 % level (5 ms fade-out, 5–10 ms restart silence, 5 ms fade-in); no click.
  - Effective rate per block reported (`eff_rate`, monotonic `wall_frames`, `src_micro`, 64-entry segment queue); RateSession 1.1045 vs 1.1042 expected.
  - Position accuracy vs click train through 4 restarts: mean 1.2 ms, max 1.9 ms over 51 clicks.
  - Stretch-to-stretch `set_rate` without flush works (≤ 29 ms position error) but can't enter/leave bypass.

options compared:
- Signalsmith Stretch 1.3.1 — recommended — MIT, ~1 % core, 0.9 MB, no allocations after construction, best tonal fidelity; soft drum onsets at slow rates; 120 ms algorithmic lag handled by the restart.
- Rubber Band 4.0.0 R3 — not adopted — GPL-2.0-or-later or commercial; crispest transients, 7–8× CPU, 7× memory; fallback only via a PO licence decision.
- Rubber Band R2 — rejected — poor on steady tones (untuned), GPL.
- WSOLA (`wsola` 0.3.0, pure Rust, MIT) — second choice; sharp transients, no pre-echo, 4–5× CPU, highest spectral flux on dense material.
- Signalsmith block 2048 — tuning option (hat rise 8.3 ms at 0.75x, 46 ms latency, worse tonal fidelity).
- `timestretch` 0.16 (MIT) — not measured (README only).
- Flush-and-restart vs short ring with plain `set_rate`: 13 ms response + 15 ms dip vs ≥ 160 ms delay and no bypass transitions.

licenses:
- Signalsmith Stretch headers and signalsmith-linear — MIT.
- `signalsmith-stretch` crate 0.1.3 — MIT (dasp MIT OR Apache-2.0); not recommended (bindgen needs libclang; bindgen BSD-3-Clause, clang-sys Apache-2.0, build-time only).
- `wsola` 0.3.0 — MIT.
- `rubberband` / `rubberband-sys` (4.0.0) — GPL-2.0-or-later or commercial (Breakfast Quay) — FLAG: measured only.
- `timestretch` 0.16 — MIT — not used.
- rtrb, rustfft, thiserror, cc — MIT/Apache-2.0; hound — Apache-2.0.

risks:
- Quality judged objectively on synthetic material only; PO must audition out/*.wav (hats at 0.75x).
- ~15 ms dip per rate change; drags must be coalesced (≤ one flush per 100–150 ms, final value applied). Two-ring crossfade possible later.
- The `signalsmith-stretch` crate needs libclang; this machine has none — the spike extracted libclang.dll from a Python wheel (PyPI `libclang` 18.1.1). Production needs our own wrapper.
- Lag formula validated for Signalsmith default only.
- CPU at 96 kHz extrapolated (~2×); mono, 5.1, mobile untested.
- Not built/measured: rubato interplay, gapless boundaries while stretched, exclusive mode, real device latency on position error.
- Quality metrics are relative constructions; Signalsmith at 1.0 is not bit-exact, so bypass at 1.0 is a policy.
- Rubber Band numbers from a built-in-FFT build with default options.
- `spikes/T-0003-audio-backend/PO-RUNBOOK.md` shows as modified (CRLF artifact), not committed.

draft ADR: spikes/T-0009-time-stretch/ADR-draft.md (written as ADR 0006). Raw results in spikes/T-0009-time-stretch/results/. Prototype standalone (empty `[workspace]`). Commit 86f391a on t/T-0009-spike-time-stretch.

follow-up tickets suggested:
- P1: Rate control in tinwave-playback (stretch stage in the decoder thread, bypass at 1.0, segment queue, eff_rate/wall_frames/src_micro, flush-and-restart with 5 ms fades, CI tests for allocations, position accuracy, RateSession) | size: M
- P1: Vendored Signalsmith Stretch wrapper (cc or cxx, pinned MIT headers + licence files, no libclang) | size: S
- P2: Coalescing of rate gestures into RateChanged and engine flushes | size: S
- P2: Listening-test spike for slow-rate transients | size: S
- P3: Gapless crossfade variant of the rate change (second ring) | size: M
- P3: Rate-change integration with the resampler (T-0019), 96 kHz and mono CPU | size: S

questions for lead / PO:
- PO: audition out/*.wav — are Signalsmith's softer hats at 0.75x acceptable, or should Rubber Band's licence be considered?
- PO: is a ~15 ms dip per rate step acceptable for v1?
- Lead: renumber the ADR once T-0003's lands as 0005?
- Lead: is a vendored C++ header plus thin cc/cxx wrapper acceptable in tinwave-playback?

---
Lead review (2026-11-02): branch touches only spikes/T-0009-time-stretch/ (18 files, no binaries, no GPL source, no
DLLs). Accepted as ADR 0006 (lead decisions 1–6 in the ADR); vendored headers + `cc` wrapper accepted; Rubber Band out
unless the PO listening pass rejects Signalsmith. The extracted libclang.dll lives only in temp folders.
Follow-ups T-0026, T-0027; P-0052 → T-0027/T-0005; P-0053 → PO; P-0054 parked; P-0055 → T-0019.
