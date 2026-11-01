# Audio analysis

Per-track acoustic features computed locally. They power similarity, energy arcs, and — crucially — the
**section map** that lets us know *where in a song* the user turned it up, seeked back, or skipped.

Lives in `tinwave-analysis`. Pure Rust preferred (decode with Symphonia; FFT with `rustfft`).
FFmpeg only as an optional fallback decoder for formats Symphonia cannot handle.

## Features (`track_features` in taste.db)

| Feature | Method (starting point) | Used for |
|---|---|---|
| `duration_ms` | decode | normalization |
| `loudness_lufs`, `loudness_range_lu`, `true_peak_db` | EBU R128 (`ebur128` crate) | Perceived level, ReplayGain, auto-loudness |
| `bpm`, `bpm_confidence` | Onset envelope + autocorrelation / tempogram | Energy arcs, workout matching (mobile cadence) |
| `key`, `mode` | Chroma + Krumhansl key profiles | Harmonic transitions in radio |
| `energy` (0–1), `energy_curve` (1 value / 2 s) | RMS + spectral flux | Energy arc, section detection |
| `brightness` | Spectral centroid mean | "Bright vs dark" taste, weather matching |
| `onset_density` | Onsets per second | Busy vs calm |
| `vocal_presence` (0–1) | Harmonic/percussive separation + mid-band energy heuristic | Focus sessions prefer low vocals |
| `intro_ms`, `outro_ms` | First/last major section boundary | Intro tolerance, hook starts, crossfade |
| `sections[]` | Self-similarity matrix + novelty curve → boundaries; label by relative energy/repetition (`Chorus` = most repeated high-energy segment, `Drop` = sharp energy jump after `Build`) | Section-aware signals |
| `fingerprint` | Chromaprint-compatible (optional, for identity matching) | Cross-source identity |

Labels are heuristic. That's fine: we need *stable* section indexes per track more than musically perfect names.

## Scheduling

- Runs only when: on AC power, system idle ≥ 2 min or the user clicked "Analyze now", and no scan is running.
- Bounded pool: 1 worker by default (configurable up to cores/2), low thread priority.
- Order: currently queued tracks → recently played → favorites → rest of library.
- Resumable: results keyed by content hash (`blake3` of audio frames, cached via file size+mtime), so moved
  files are not re-analyzed.
- A track played before analysis still logs `pos_ds`; sections are joined later. Never block on analysis.

## Mapping signals to sections

At write time, if features exist, events include `section`/`section_index`. If not, the taste engine
joins `pos_ds` against `sections[]` when computing aggregates. Per track we maintain:

- `replay_heatmap[64]` — seek-back targets and repeated listening
- `volume_up_heatmap[64]`
- `skip_heatmap[64]` — where people bail
- `hook_section` — argmax of combined replay + volume-up heat, once ≥ 3 independent plays agree

Heatmaps are `u8` bins, sparse-encoded, created only after ≥ 3 relevant events, and decayed/renormalized
so they never overflow. Heatmaps are updated before raw events are compacted away.

## Storage

Fixed ~64-byte feature struct per track, energy curve as one `u8` per 2 s, ≤ 16 sections × 4 bytes.
About 250 bytes per analyzed track (≈ 12 MB for a 50 000-track library). Intermediate data (spectrograms,
chroma, self-similarity matrices) is never persisted.
