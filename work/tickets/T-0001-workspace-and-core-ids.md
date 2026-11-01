---
id: T-0001
title: Create Cargo workspace, quality gates, and core id types
type: chore
epic: E-00
status: in_progress
priority: P0
size: M
depends_on: []
scope:
  - Cargo.toml
  - rust-toolchain.toml
  - deny.toml
  - rustfmt.toml
  - clippy.toml
  - .gitignore
  - crates/tinwave-core/
owner:
branch: t/T-0001-workspace-and-core-ids
review_round: 0
created_by: lead
---

## Context
Everything else builds on this. Layout and rules: docs/rules/architecture.md, docs/rules/rust.md,
docs/rules/storage-budget.md (compact ids), docs/guides/desktop-dev-setup.md.

## Scope
- Root virtual workspace `Cargo.toml` with `[workspace.dependencies]`, `members = ["crates/*", "apps/*"]`,
  `exclude = ["spikes/*"]` so spike prototypes never join the workspace.
- `rust-toolchain.toml` pinning a current stable with rustfmt + clippy.
- `deny.toml` allowing MIT/Apache-2.0/BSD/ISC/Zlib/Unicode licenses; deny GPL/AGPL by default; advisories on.
- `.gitignore` for Rust, Qt build dirs, local data dirs.
- `crates/tinwave-core` with `#![forbid(unsafe_code)]` and newtype ids: `RecordingId`, `SourceId`, `SessionId`,
  `DeviceId` (UUIDv7-backed), `SignalEventId { device: DeviceId, seq: u64 }`; `Tier` enum (T0–T3);
  time helpers `UtcMillis(i64)` and `TzOffsetMin(i16)`.

## Out of scope
- Any other crate. DB, signals, UI.

## Acceptance criteria
- [ ] `cargo build --workspace` and `cargo test --workspace` pass — verified by command output.
- [ ] `cargo fmt --all --check` and `cargo clippy --workspace --all-targets -- -D warnings` pass.
- [ ] `cargo deny check` passes with the license policy above.
- [ ] Ids: generation is time-ordered (UUIDv7) — unit test; `SignalEventId` orders by `(device, seq)` — unit test.
- [ ] Ids implement `Copy, Eq, Ord, Hash, Debug, Display` and serde round-trip — unit tests.
- [ ] No `unwrap`/`expect` outside tests.

## Signals & storage
None.

## Notes for the builder
Keep `tinwave-core` dependency-light (`uuid`, `serde`, `thiserror`). No async runtime.
