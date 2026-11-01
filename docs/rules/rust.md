# Rust rules

## Baseline

- Edition 2021+, stable toolchain pinned in `rust-toolchain.toml`.
- `cargo fmt`, `cargo clippy -D warnings` clean. `#![forbid(unsafe_code)]` in every crate except
  where FFI is needed (`tinwave-context` OS modules, playback output, bridge). Each `unsafe` block has a `// SAFETY:` comment.
- Workspace-level dependency versions in root `Cargo.toml` (`[workspace.dependencies]`).

## Errors

- Library crates: `thiserror` enums, one per crate, meaningful variants. No `anyhow` outside binaries/tests.
- No `unwrap()`/`expect()` in non-test code unless the invariant is proven locally and commented.
- Never swallow an error. If a degraded path is intended, log at `warn` with context and return a typed `Degraded` state.

## Types

- Newtype ids: `RecordingId(Uuid)`, `SourceId`, `SessionId`, `DeviceId`. Signal events are identified by
  `SignalEventId { device: DeviceId, seq: u64 }`; inside `signals.db` all of these map to small integer
  surrogates (see [storage-budget.md](storage-budget.md)).
- Signal payloads are small quantized structs (`u8` volume, `u8` rate, `u16` flags), not `f64`s or strings.
- Time: store `i64` UTC milliseconds + `i16` tz offset minutes. Use `time` or `jiff`; never naive local time in storage.
- Durations/positions in `u32`/`u64` milliseconds with `_ms` suffix.
- Units in names: `volume_linear` (0.0–1.0), `gain_db`, `loudness_lufs`, `rate` (1.0 = normal).

## Concurrency

- Bounded channels only (`crossbeam_channel::bounded`, `tokio::sync::mpsc` with capacity).
- When a signal channel is full: drop the oldest *low-priority* event (browse dwell, hover) and count the drop;
  never block the producer, never drop T0 playback events (they go to a reserved slot).
- Everything long-running takes a `CancellationToken`.
- No locks held across `.await` or across audio callbacks.

## Logging

- `tracing` with structured fields. Never log credentials, tokens, full file paths at `info`+, or raw signal payloads.
- Log volume and seek events at `trace` only.

## Performance

- No whole-library `Vec`s. Paginate / stream from the DB.
- Hot path (signal emit) must not allocate more than one small struct; payloads are small enums, not JSON strings.
- Benchmarks with `criterion` for: signal emit, batch write, taste scoring of 1 000 candidates.
