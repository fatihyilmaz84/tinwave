---
id: T-0004
title: Signal schema, tier mapping, and packed payload codec
type: feature
epic: E-00
status: in_review
priority: P0
size: M
depends_on: [T-0001]
scope:
  - crates/tinwave-signals/
  - Cargo.toml            # [workspace.dependencies] only — members glob "crates/*" already covers the crate
  - Cargo.lock
owner: tw-builder (round 2)
branch: t/T-0004-signal-schema-codec
review_round: 2
created_by: lead
---

## Context
docs/guides/signal-catalog.md (all kinds), docs/rules/storage-budget.md §3 (compact payloads),
docs/rules/privacy-and-signals.md (tiers), docs/guides/adding-a-signal.md (payload evolution).

## Scope
- `SignalKind` enum with explicit, stable discriminants for every **E**-class signal in the catalog; `kind.tier()`.
- `StorageClass` (`Event | Counter | Derived`) per kind.
- Payload structs per kind with quantized fields (`u8` volume, `u8` rate, `u8` fraction, `u16` flags,
  `InputSource`, `Section`), and an encode/decode codec to a compact byte form with `payload_version`.
- `SignalEvent` struct matching `signal_event` columns in docs/rules/database.md.

## Out of scope
Bus, storage, emission from playback.

## Acceptance criteria
- [ ] Every E-class catalog signal has a kind, tier and payload — test lists kinds and compares to a fixture
      derived from the catalog.
- [ ] Discriminants are stable: a snapshot test fails if any value changes.
- [ ] Codec round-trips all payloads — `proptest`.
- [ ] Encoded payload ≤ 12 bytes for every kind (assert in test); empty payloads encode to `None`.
- [ ] Decoder accepts every historical `payload_version` (only v1 now; test structure in place).

## Signals & storage
Defines the schema; storage target ≤ 60 bytes/event depends on these sizes.
