# Testing rules

## Pyramid

1. **Unit tests** next to code (`#[cfg(test)]`) for all domain logic: identity matching, file-op planning,
   score functions, decay math, bucket backoff, explanation generation.
2. **Property tests** (`proptest`) for: sync conflict resolution, event dedupe, rollup correctness
   (rollup(raw) == aggregate(raw)), file-op journal recovery.
3. **Integration tests** per crate in `tests/` with a temp SQLite DB and fixture audio files
   (`fixtures/audio/` — short, CC0/self-made clips in FLAC, MP3, M4A, ALAC, WAV).
4. **Replay tests** for recommendations: feed a recorded JSONL signal log (`fixtures/signals/*.jsonl`) through
   `tinwave-taste` + `tinwave-radio` with a fixed seed and clock; assert top-N and explanation strings
   (golden files via `insta`).
5. **Real-world tests**: real Jellyfin/Plex instances, real devices, real optical drives (spec §18).

## Signals must be tested

Every signal kind needs:
- an **emission test**: the triggering action produces exactly one event with the right payload, position and input source;
- an **interpretation test**: the score/weight it produces in common and edge contexts
  (e.g. volume-down during `VideoCall` foreground ⇒ no negative score);
- a **privacy test**: when its category is disabled, nothing is written and the Signal Center shows it as off.

## Playback must be tested (ADR 0005)

- **Gapless sweep test:** a generated −20 dBFS sweep split into two files at a non-codec-aligned frame, per
  format (WAV, FLAC, ALAC, MP3 with LAME tag; M4A with trim); the recorded renderer output must show a 0-frame gap.
- **Seek test:** output after a seek matches the reference from exactly the requested frame.
- **RT allocation test:** a counting global allocator proves zero allocations/frees inside the render callback
  across play, seek, volume and device changes.
- Test audio is generated or CC0/self-made; never commit copyrighted music.

## Determinism

- Inject `Clock`, `Rng` and `ContextProvider`s everywhere. No `SystemTime::now()` or `thread_rng()` in domain code.
- Weather and other network providers have fake implementations used in all tests.

## Performance budgets (CI benchmarks, fail on > 20 % regression)

| Operation | Budget |
|---|---|
| Signal emit (producer side) | < 50 µs |
| Batch write 256 events | < 5 ms |
| Signal writer extra RSS | < 2 MB |
| Context snapshot (all T0–T2 providers) | < 1 ms total |
| Score 1 000 radio candidates | < 20 ms |
| Next-track pick | < 50 ms |
| Audio analysis | paused on battery; ≤ 1 core when running |

## Storage growth test (CI, required)

A synthetic listener generator (`crates/tinwave-db/tests/growth/`, run by `cargo test -p tinwave-db --release --test
storage_growth -- --nocapture`) simulates the reference heavy user
from [storage-budget.md](storage-budget.md) — 8 h/day, ~120 tracks/day, 50 000-track library, realistic
volume/seek/skip/browse rates — for **5 simulated years**, running compaction as it would in real life.
It fails if:
- `signals.db` > 60 MB or `taste.db` > 80 MB at any point,
- average raw event > 60 bytes on disk,
- context snapshots exceed ~60 distinct per simulated day,
- any signal group exceeds its bytes/day estimate in the signal catalog by > 50 %.

Also test: compaction is idempotent and resumable after a crash mid-batch; taste scores computed before and
after compaction differ by < 1 %.

Plus the spec §12 measurements (startup, search latency, memory peaks, idle CPU) on all three desktop OSes.
