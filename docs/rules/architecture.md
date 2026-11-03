# Architecture rules

## Workspace layout

```
crates/
  tinwave-core        Domain types & ids (Artist, Recording, MediaFile, PlaybackSource…), Clock/Rng seams.
                      No IO beyond the system clock/RNG seams (SystemClock, SystemRng), no async runtime.
  tinwave-db          SQLite (rusqlite), migrations, FTS5, repositories. Only crate that writes SQL.
  tinwave-library     Scanning, tag read/write, file organization (journal, dry-run, recovery).
  tinwave-playback    Playback engine (Symphonia decode → ring → RT renderer), queue, outputs
                      (output/windows = WASAPI, output/cpal), resampler, time-stretch. Emits PlaybackEvents.
                      The render callback never allocates, locks or does IO (ADR 0005).
  tinwave-signals     Signal event schema, bus, batched writer, Signal Center queries.
  tinwave-context     Context providers: clock, holidays, sun, power, idle, audio device,
                      foreground-app category, network place, weather. All OS code for context lives here,
                      together with the ContextProvider trait (T-0014).
  tinwave-test-alloc  Dev-only: per-thread counting allocator for RT "no allocation" tests. Never a normal dependency.
  tinwave-analysis    Offline audio DSP: BPM, key, loudness, energy, sections, fingerprints.
  tinwave-taste       Aggregation, implicit scores, context buckets, similarity, explanations.
  tinwave-radio       Smart Radio sessions, smart playlists, auto-stations.
  tinwave-connectors  Jellyfin, Plex, Navidrome, Emby, Subsonic (Phase 6).
  tinwave-sync        Pairing, protocol, transfer (Phase 3/5).
apps/
  tinwave-desktop     Qt 6 Quick/QML + CXX-Qt bridge. Thin: view models only.
```

## Dependency direction

```
tinwave-desktop
   └─> tinwave-radio ─> tinwave-taste ─┬─> tinwave-signals ─> tinwave-core
                                       └─> tinwave-analysis ─> tinwave-core
   └─> tinwave-playback ─> tinwave-signals (emits)          tinwave-context ─> tinwave-signals
   └─> tinwave-library ─> tinwave-db ─> tinwave-core
```

- Arrows only point downward. `tinwave-core` depends on nothing in the workspace.
- `tinwave-signals` defines traits (`SignalSink`, `ContextProvider`, `SignalStore`); `tinwave-db` implements the store.
- `tinwave-playback` knows nothing about taste. It emits facts; others interpret.
- Inside `tinwave-playback`, sample-rate conversion happens **only in the decoder thread**
  (`reader -> [stretch] -> [resample] -> ring`, ADR 0005 decision 5, ADR 0006): the render callback only ever sees the
  stream rate. Same-rate tracks share one continuous resampler (no filter restart at a boundary). Dither is applied
  only at the output conversion, and only for integer formats (`output/convert.rs` has the policy).
- `tinwave-desktop` wires everything in one composition root (`app/wiring.rs`). No global singletons elsewhere.

## Boundaries

- **OS-specific code** only in `tinwave-context`, `tinwave-playback` (output) and `tinwave-desktop`, behind
  traits with `#[cfg(target_os = "...")]` modules: `windows.rs`, `macos.rs`, `linux.rs`. Each impl returns
  `Unknown`/`Unsupported` rather than panicking when a capability is missing.
- **Threads:** one playback thread (realtime-ish, never allocates in the audio callback), one signal writer
  thread, a bounded worker pool for scanning/analysis, the Qt UI thread. Async runtime (tokio) only for
  network-facing crates (weather, connectors, sync).
- **Events, not calls, flow upward.** Playback → `SignalBus` → writer + live taste updates + UI.
- **Capabilities are declared.** Connectors and context providers report what they support; the UI hides the rest.

## Identity

Separate musical identity from files and server items (spec §4.7, §5). Signals reference `recording_id`
**and** `source_id` so the same song across local and server sources accumulates one taste history while
keeping provenance.
