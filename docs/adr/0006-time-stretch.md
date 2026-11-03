# ADR 0006 — Pitch-preserving playback rate: Signalsmith Stretch in the decoder thread, rate change as flush-and-restart

- **Status:** Accepted (lead, from spike T-0009; report work/reports/T-0009/spike-1.md). Audio quality at slow rates to be confirmed by a PO listening pass before E-01 acceptance.
- **Date:** 2026-11-02

## Context

The signal catalog (section 2) needs a 0.75x to 1.5x playback rate with pitch preservation, and `RateChanged` /
`RateSession` signals. The license rule prefers MIT/BSD (`deny.toml`: MIT, Apache, BSD, ISC, Zlib, Unicode); GPL needs an
ADR plus a PO decision. The engine from T-0003 (decoder thread -> `rtrb` ring -> RT renderer, 0 allocations in the
callback) is the integration target: a stretcher must never run inside the callback.

Measured on **Windows 11 Pro 10.0.26200, AMD Ryzen 7 7800X3D, Rust 1.92 release, MSVC 14.50**, 44.1 kHz stereo f32,
one pinned core, 1024-frame blocks. Test material is generated (`src/synth.rs`): 120 BPM drums from noise bursts,
a four-chord detuned-saw pad, and a 0.5 s-per-note pure-sine melody; no real music was used and nobody listened.
Quality is judged by objective measures only (listed in the spike results); WAVs for a human listening pass can be
regenerated with `t0009-time-stretch-spike quality wav` (about 100 MB in `out/`, git-ignored). Raw output is in
`spikes/T-0009-time-stretch/results/`.

Options measured: Signalsmith Stretch 1.3.1 (via `signalsmith-stretch` 0.1.3, three configurations), Rubber Band 4.0.0
(R3 "finer" and R2 "faster" engines, real-time mode, via `rubberband-sys` 0.1.4, built-in FFT), and WSOLA (`wsola`
0.3.0, pure Rust). The pure-Rust `timestretch` 0.16 crate (MIT, a young DJ-oriented session API) was read but not measured.

| Option (all at 44.1 kHz) | CPU, one core, realtime playback at 0.75 / 1.25 / 1.5 | Memory per instance | Latency (click lag, rate 1.0) | Pitch error median / max, cents | Hat rise time at 1.25 (source 2.0 ms) | Pre-echo before onset | License |
|---|---|---|---|---|---|---|---|
| Signalsmith default preset | 1.1 / 1.4 / 1.5 % | 0.9 MB | 120 ms (140 ms at 0.75, 100 ms at 1.5) | 0.09 / 1.6 | 5.8 ms | -35 dB | MIT |
| Signalsmith cheaper preset | 1.0 / 0.9 / 0.9 % | 0.8 MB | 140 ms | 0.12 / 1.6 | 5.6 ms | -38 dB | MIT |
| Signalsmith block 2048 / interval 512 | 1.1 / 1.2 / 1.2 % | 0.3 MB | 46 ms | 0.10 / 1.2 | 4.8 ms | -44 dB | MIT |
| Rubber Band R3 finer | 8.1 / 9.6 / 11.6 % | 6.8 MB | about 0 (compensated) | 0.09 / 9.6 | 3.9 ms | -41 dB | GPL-2.0-or-later or commercial |
| Rubber Band R2 faster | 2.7 / 2.6 / 3.1 % | 8.1 MB | about 0 | 4-8 / 29 | 15.1 ms | -45 dB | GPL-2.0-or-later or commercial |
| WSOLA (15 ms hop) | 5.9 / 5.9 / 6.0 % | 1.0 MB | 7-13 ms | 0.08 / 0.6 | 3.0 ms | none | MIT |

Other measures that decided it (full table in `results/quality.txt`): on the stationary pad, log-spectral distance to the
ideal stretched pad is best for Signalsmith default (6.7 to 7.7 dB, WSOLA 7.5 to 8.2, Rubber Band R3 8.6 to 8.9, the
2048-block Signalsmith 10 to 12); on a steady 1 kHz sine the tonal purity is 41.6 dB for Signalsmith default and WSOLA,
39 to 41 dB for R3, and R2 collapses to 4 to 13 dB with 36 to 95 % amplitude wobble (R2 was not tuned; it is a comparator
only); stereo correlation of the pad is preserved by all (|delta| < 0.02); level change is -0.07 to -0.19 dB for
Signalsmith default. Signalsmith softens drum onsets at 0.75x (hat rise 13.8 ms, i.e. noticeably dull hats) and is
fine at 1.25x to 1.5x; WSOLA keeps onsets sharp but is the weakest on dense polyphonic material by flux and costs
4 to 5x the CPU of Signalsmith.

## Decision

1. **Library: Signalsmith Stretch (MIT).** It is the only option that is MIT, about 1 % of one core, 0.9 MB, allocation
   free after construction (0 heap allocations in 60 s of processing, counted for Rust and for C++ `operator new`), and
   best on tonal material. Use `preset_default` (or `preset_cheaper` if CPU or latency matters later); expose the
   preset as an internal constant, not a user setting.
2. **Bindings: our own thin C++ wrapper built with `cc`, not the `signalsmith-stretch` crate.** The crate runs `bindgen`
   at build time, which needs `libclang.dll`; the dev machine does not have it (the spike had to extract it from a
   Python wheel). The upstream library is header-only; the crate's wrapper is 133 lines. Vendor the pinned MIT headers
   (`signalsmith-stretch` 1.3.x and `signalsmith-linear`, both MIT) in `tinwave-playback` with their license files and
   hand-write the `extern "C"` declarations (or use `cxx`, which Tinwave already brings through CXX-Qt).
3. **Where it runs: the decoder thread, never the callback.** Chain: `reader -> [stretch] -> [resample] -> ring`. At
   rate exactly 1.0 the stretcher is bypassed, so playback stays bit-exact and costs nothing. The stretcher is driven with
   a fixed number of output frames per call and `round(out * rate)` input frames (carry the fraction), so the
   in/out ratio, and therefore the source position, is exact by construction.
4. **A rate change is a flush-and-restart at the audible source position** (the T-0003 seek handshake):
   control sets the atomic target rate and bumps `flush_req`; the renderer fades out at most 5 ms of already queued
   audio, drops the rest of the ring and publishes how far the source advanced; the decoder computes the audible source
   position from that, reopens the reader there, and for rate != 1.0 resets the stretcher, feeds from that position and
   discards the first `lag_out(rate) = out_latency + in_latency / rate` output frames (about 1 ms of CPU, much faster than
   realtime) so the first delivered frame is the audio at the audible position; the renderer fades the new stream in over
   5 ms. The ring stays 1 s so stalls are still absorbed.
5. **Effective rate is an engine output, not a UI guess.** The decoder emits a tiny `(out_frame, rate)` segment record
   (second SPSC queue, 64 entries, pushed only when the rate changes). The renderer does O(1) bookkeeping per block,
   publishes the block's effective rate (`eff_rate`, atomic) and two monotonic counters, `wall_frames` and
   `src_micro` (source micro-frames advanced). `RateSession.time_weighted_rate = src_frames / wall_frames`;
   audible source position = `base + src_micro_since_flush` (+ device latency, as in T-0003).
6. **Order with the resampler:** stretch at the file's rate, then rubato to the device rate; the segment rate is
   `stretch_rate * file_rate / device_rate` source frames per output frame. Not built in the spike.
7. **Rubber Band and WSOLA are not adopted.** Rubber Band is GPL-2.0-or-later (or a commercial licence from Breakfast
   Quay, price not checked) and costs 7 to 8x the CPU and 7x the memory of Signalsmith at the same rate; its advantage is
   crisper transients (about 2 ms shorter hat rise time at 1.25x, 8 ms shorter at 0.75x). If listening tests show the
   slow-rate softness is unacceptable, the first thing to try is a smaller Signalsmith block (measured: block 2048 gives
   hat rise 8.3 ms at 0.75x and 46 ms latency, but worse tonal fidelity), then a PO decision on Rubber Band.

## Consequences

- Easier: one stage, one thread, no new RT code; bypass at 1.0 keeps the common case untouched; effective rate and
  position come from the same counters the renderer already needs.
- Rate change latency is 10 to 19 ms (median 13 to 17; one earlier run had a 41 ms outlier) from `set_rate()` to the first block at the new rate, measured
  with a simulated 10 ms device period, plus the real device latency from T-0003 (about 22 ms on WASAPI shared). The
  alternative of keeping a short ring and just calling `set_rate` on the stretcher has no dip but delays the change by
  the ring fill (160 ms at a 150 ms target, 994 ms with the 1 s ring), and cannot switch between bypass and stretch.
- A rate change makes a short dip: 5 ms fade-out, about 5 to 10 ms of silence while the decoder restarts, 5 ms
  fade-in (about 15 ms below 70 % level measured on a sine, no click: largest sample step equals the steady sine's).
  Fine for button-style steps; a slider drag must be coalesced (at most one flush per 100 to 150 ms, final value
  always applied). A gapless variant would use the T-0003 `Reconfigure` second ring and cross-fade old and new streams
  in the renderer; deferred until a listening test says the dip matters.
- Position accuracy through four rate changes (1.0 -> 1.25 -> 0.75 -> 1.5 -> 1.0) against a click train with known source
  positions: mean 1.2 ms, max 1.9 ms over 51 clicks (zero-latency simulated device). Time-weighted rate matches the
  wall-clock schedule to 0.0004.
- We maintain a vendored C++ header and a 100-line wrapper; the libclang requirement disappears.
- Added code size: the Signalsmith static library is 0.7 MB of object code before linking; Rubber Band's is 3.8 MB.
- CPU scales with sample rate: expect about 2x at 96 kHz (extrapolated, not measured). Mono and 5.1 are untested;
  the spike is stereo only like the T-0003 engine.
- Not covered: listening tests on real music (the PO should audition `out/*.wav`, especially hats at 0.75x), gapless
  track boundaries while stretched (the next track must start a fresh stretcher or flush the tail), rate changes during
  an exclusive-mode stream, and mobile CPU budgets.

## Lead decisions (2026-11-02)

1. **Signalsmith Stretch (MIT) is adopted**, run in the decoder thread, bypassed at exactly 1.0×, rate change =
   flush-and-restart at the audible source position with 5 ms fades.
2. **Vendored headers + thin `cc` wrapper** in `tinwave-playback` (pinned version, MIT licence files kept next to the
   headers). The `signalsmith-stretch` crate is not used: its bindgen step needs libclang at build time.
3. **Rubber Band (GPL-2.0+/commercial) stays out.** Revisiting it needs a PO licence decision, only if the listening
   pass rejects Signalsmith's slow-rate transients.
4. The ~15 ms dip per rate step is accepted for v1, subject to the PO listening pass; a two-ring crossfade is parked.
5. Rate drags: the engine applies at most one flush per 100–150 ms during a drag and always applies the final value;
   the `RateChanged` signal stays one event per gesture (600 ms coalescer, T-0005).
6. Stretch runs at the file rate, before the rubato resampler (T-0019): effective stretch rate = rate.

## Implementation (T-0027, 2026-11-03)

Built as decided, with these refinements (review: work/reports/T-0027/review-1.md):

1. **No segment queue (replaces decision 5's second SPSC queue).** A rate change is always a flush, so the ring only
   ever holds one rate. The decoder stores `Shared::stream_rate` while the ring is empty (after the renderer's flush
   ack, before the first push); the renderer reads it after popping and books `eff_rate`, `wall_frames` (= consumed)
   and `src_micro`. The rate is in stream frames per output frame, so decision 6's `file_rate / device_rate` factor is
   not needed. The counters are exposed as `RateStats`.
2. **Positions come from `Mark.rate`** (replaces "audible position = base + src_micro_since_flush"). Marks carry the
   rate; a rate restart appends a `MarkKind::Rate` mark and keeps older marks until consumed, so positions inside the
   device buffer and a restart landing in the next track stay correct.
3. **Pre-roll.** A freshly reset stretcher overshoots the level by up to 4-5 % for about 60 ms, so the reader starts
   `2 x input_latency` frames early (`TrackReader::seek_with_preroll`) and the discard grows by `round(preroll / rate)`.
4. **The fade-out spans callbacks**, so a device buffer shorter than 5 ms still gets the full ramp.
5. **Throttle: one restart per 125 ms** from flush start; changes in between wait and the latest value wins. Nothing
   audible (nothing loaded, or everything played) adopts the rate without a flush.
6. **Click criterion restated.** A stretched sine's own steepest sample step is up to about 0.5 % above a pure sine's;
   the step at a restart is no larger than the steady stretched signal's (measured 1.0035x vs 1.0053x of the analytic
   sine). The Consequences sentence "largest sample step equals the steady sine's" means this.
7. **Latency is not re-measured.** The 10-19 ms set_rate-to-audio figure above is the spike's (no pre-roll, no flush
   backoff). The engine's flush wait backs off up to 16 ms, so the real dip may be longer; it is measured in the PO
   listening pass (P-0251) and the backoff fix is T-0063. Do not quote it as a measured engine figure.
8. `RateStats` counters are booked at the callback, not at the speaker: a per-play time-weighted rate taken from
   snapshots at play start/leave (T-0062) is off by about one device latency at the edges.

## Spec / docs updated

On acceptance, in the same change:
- `docs/guides/signal-catalog.md` section 2: replace "license check" sentence with the decision (Signalsmith Stretch,
  MIT, range 0.75x to 1.5x, bypass at 1.0, `RateSession` is computed from the engine's `src_frames / wall_frames`).
- `docs/rules/architecture.md`: add "time-stretch and resampling run in the decoder thread; the callback only applies
  fades and rate bookkeeping".
- Engine ticket (follow-up of T-0016): add the segment queue, `eff_rate`/`wall_frames`/`src_micro` counters, the
  flush-to-audible-position restart, and the 5 ms fades to the engine contract and its CI tests.
- `deny.toml`: nothing to add (MIT only).
- Spec section 7 (playback): mention playback-rate control and its limits.
