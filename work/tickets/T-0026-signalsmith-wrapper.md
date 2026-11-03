---
id: T-0026
title: Vendored Signalsmith Stretch with a thin cc wrapper (no libclang)
type: chore
epic: E-01
status: in_review
priority: P1
size: S
depends_on: [T-0016]
scope:
  - crates/tinwave-playback/vendor/signalsmith/    # pinned MIT headers + licence files
  - crates/tinwave-playback/src/stretch/           # wrapper module
  - crates/tinwave-playback/build.rs
  - crates/tinwave-playback/Cargo.toml
  - Cargo.lock
owner: tw-builder
branch: t/T-0026-signalsmith-wrapper
review_round: 1
created_by: from P-0050
---

## Context
ADR 0006 (spike T-0009). The `signalsmith-stretch` crate needs libclang via bindgen; we vendor the MIT C++ headers
(Signalsmith Stretch 1.3.1 + signalsmith-linear) and expose a minimal C ABI through `cc` (MSVC on Windows).

## Scope
- Pinned headers with their licence files and a README naming version and source URL.
- `build.rs` compiles one small C++ file exporting create/configure/process/reset/latency; safe Rust wrapper in
  `src/stretch/` (the only `unsafe` in the crate's library, documented; library keeps `#![forbid(unsafe_code)]`
  everywhere else via module-level allow).
- No allocation after construction (verified with the T-0016 allocation test helper).

## Acceptance criteria
- [ ] Builds on Windows with only MSVC + Rust (no libclang, no Python) — clean build log.
- [ ] Round trip: 1.25× stretch of a sine keeps pitch within 2 cents and length within 1 block — test.
- [ ] Zero allocations in `process` after construction — test.
- [ ] `cargo deny check` passes; licence files present.

## Signals & storage
None.
