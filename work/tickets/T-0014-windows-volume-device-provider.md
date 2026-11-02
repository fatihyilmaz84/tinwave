---
id: T-0014
title: Windows output-device and system-volume providers in tinwave-context
type: feature
epic: E-01
status: done
priority: P1
size: M
depends_on: []          # T-0005, T-0013, T-0015 done
scope:
  - crates/tinwave-context/
  - Cargo.toml            # [workspace.dependencies] only
  - Cargo.lock
owner: tw-builder (round 1)
branch: t/T-0014-windows-volume-device-provider
review_round: 2
created_by: from P-0007
---

## Context
ADR 0004 (from spike T-0008; prototype in `spikes/T-0008-win-volume/`). Signal catalog §1 `SystemVolumeChanged`,
`MuteToggled`, §6 `OutputDeviceChanged`. docs/guides/context-engine.md (provider trait, capability, snapshot),
docs/rules/architecture.md (crate boundaries), rust.md (concurrency, `unsafe` rules for COM), privacy-and-signals.md
(no global input hooks), storage-budget.md (coalescing). D-0001: Windows first; other OSes return `Unsupported`.

## Scope
- Provider thread (COM MTA) owning `IMMDeviceEnumerator`, `IMMNotificationClient` and one
  `IAudioEndpointVolumeCallback` on the bound render endpoint; callbacks only timestamp, copy and `send`.
- Re-bind state machine: default-device change (dedupe the 3 per-role callbacks), bound-device unplug/removal,
  failure to bind ⇒ `Unknown` fields.
- Self-vs-external attribution with a process-wide event-context GUID.
- Mute/volume diff ⇒ `MuteToggled` / `SystemVolumeChanged`; 400 ms trailing coalescing via T-0005's `GestureCoalescer`.
- Classifier `class = f(transport, form_factor, internal_jack)` per ADR 0004 rule order, as a pure function with
  unit tests over fixture tuples (including every "measured" row in the ADR table).
- `ContextProvider::snapshot()` from an `ArcSwap` cache; `capability()` = `Supported` on Windows, `Unsupported` elsewhere.
- `device_hash = blake3(device_secret || endpoint_id)`; no friendly names or container ids persisted.

## Out of scope
macOS/Linux providers (own tickets). Playback-engine volume (E-01 playback ticket). Signal Center UI.

## Acceptance criteria
- [ ] Classifier unit tests cover every ADR 0004 table row — `cargo test -p tinwave-context`.
- [ ] Coalescing: a scripted 60-step burst within 200 ms ⇒ one `SystemVolumeChanged` with correct `from`/`to`/`gesture_ms` — test with fake endpoint events and `ManualClock`.
- [ ] Mute-only change ⇒ `MuteToggled` only; scalar-only ⇒ `SystemVolumeChanged` only — test.
- [ ] Own-GUID changes produce no event — test with injected events; live check documented in the handoff.
- [ ] 3 default-change callbacks for one switch ⇒ one `OutputDeviceChanged` — test.
- [ ] Idle CPU ≈ 0 and RSS delta ≤ 4 MiB with the provider running — measurement in the handoff.
- [ ] Non-Windows builds compile and report `Unsupported` — `cargo check` with a non-Windows target or cfg test.
- [ ] fmt, clippy `-D warnings`, tests, `cargo deny check` pass.
- [ ] Before epic acceptance: PO-RUNBOOK R1–R3, R5–R9 results attached (P-0006).

## Signals & storage
Emits T0 `SystemVolumeChanged`, T0 `MuteToggled`, T1 `OutputDeviceChanged` (catalog rows as updated by ADR 0004).
Coalesced: a few events per listening hour; within the catalog §9 estimates.

## Notes for the builder
Reuse the spike's `classify.rs`, `device.rs` and `monitor.rs` ideas, but rewrite to the rules: no `unwrap`, `unsafe` only
in a small documented COM module, all COM calls on the provider thread (never inside callbacks — deadlock risk).
The spike's selftest changes the real system volume — any live test must restore the volume and mute and must
not use `panic=abort`.

## Merged
2026-11-02, merge commit f831e27 (branch head 3ee67fc). Reviews: review-1 (CHANGES_REQUESTED), review-2 (APPROVE). Gates green on main. PO-RUNBOOK rows still gate E-01 acceptance (P-0006, P-0061).
