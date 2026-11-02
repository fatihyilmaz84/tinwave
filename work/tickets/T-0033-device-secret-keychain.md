---
id: T-0033
title: Store the DeviceSecret in the OS keychain (Windows Credential Manager first)
type: feature
epic: E-01
status: done
priority: P1
size: S
depends_on: []
scope:
  - crates/tinwave-context/src/identity.rs   # or a new secret_store module next to it
  - crates/tinwave-context/src/lib.rs        # re-exports
  - crates/tinwave-context/Cargo.toml        # windows features for Credential Manager
  - crates/tinwave-context/tests/
  - Cargo.lock
owner: tw-builder (round 1)
branch: t/T-0033-device-secret-keychain
review_round: 1
created_by: from P-0062 (split from T-0024)
---

## Context
T-0014 hashes endpoint ids with `blake3(device_secret || endpoint_id)`; `DeviceSecret` is generated via
`&mut impl CryptoRng` (sealed, SystemRng only) and is not persisted yet. privacy-and-signals.md; rust.md (unsafe in a
small documented module); architecture.md (OS code in tinwave-context behind traits, Unsupported elsewhere).

## Scope
- `SecretStore` trait: load-or-create the DeviceSecret. Windows impl via Credential Manager (`CredReadW`/`CredWriteW`,
  generic credential, per-user, target name `Tinwave/device-secret`); in-memory fake for tests; macOS/Linux return
  `Unsupported` (own tickets later).
- First run creates the secret with `SystemRng` and stores it; later runs load the same bytes; a corrupt or wrong-size
  entry is a visible error (never silently regenerated — that would re-key every device hash).
- Secret bytes never logged; `Debug` redacted (already for `DeviceSecret`).

## Acceptance criteria
- [ ] Load-or-create round trip with the fake store; corrupt entry ⇒ visible error, no regeneration — tests.
- [ ] Windows impl: an `#[ignore]`d live test that writes/reads/deletes a **test-only** target name (never the real one)
      — run once and report; default `cargo test` never touches the real Credential Manager.
- [ ] `unsafe` only in one documented module with per-block SAFETY comments (`undocumented_unsafe_blocks = "deny"`).
- [ ] Non-Windows builds compile and report Unsupported; fmt, clippy `-D warnings`, tests, `cargo deny check` pass.

## Signals & storage
None (protects device hashes; T1 OutputDeviceChanged stays stable across restarts).

## Merged
2026-11-02, merge commit 8612674 (branch head 6f75257). Review: review-1 (APPROVE). Gates green on main.
