---
id: E-00
title: Feasibility & foundations
status: active
spec: §16 Phase 0, ADR 0001, ADR 0002
---

## Outcome
The risky technical choices are proven with measurements on Windows (see D-0001), the Cargo workspace and
quality gates exist, and the signal pipeline core (schema → bus → compact store) works and meets the storage budget.

## PO acceptance demo
1. `cargo test --workspace` green; clippy/fmt/deny gates in place.
2. A QML window scrolling a 100 000-row list served from Rust, with measured memory and startup.
3. A prototype playing FLAC/MP3/M4A/ALAC/WAV gaplessly, with the chosen backend recorded in an ADR.
4. A live console showing system volume and output-device changes while music plays.
5. The growth test report: 5 simulated heavy-listener years vs the 60 MB `signals.db` budget.

## Tickets
- T-0001 Cargo workspace, quality gates, and core ids — done
- T-0002 Spike: CXX-Qt + QML virtualized list, memory & startup — ready, blocked on the Qt MSVC kit (PO)
- T-0003 Spike: audio backend selection — done (ADR 0005)
- T-0008 Spike: Windows system-volume & output-device notifications — done (ADR 0004)
- T-0011 Spike: native Qt Quick styles & OS integration (design system, ADR 0003) — ready, blocked on the Qt MSVC kit (PO)
- T-0004 Signal schema and packed payload codec — done
- T-0005 Signal bus with priority drop policy and UI counters — done (+ T-0022 hardening)
- T-0006 signals.db migrations and batched SignalWriter — done (39.4 B/event)
- T-0007 Storage growth test v1 — done (90 days 4.38 MB; 5 years ~17 MB)
- T-0009 Spike: time-stretch library and license — done (ADR 0006)
- T-0010 Spike: Jellyfin connect, browse, stream — backlog (P2)

## Risks & open questions
- CXX-Qt maturity / build complexity on Windows (T-0002).
- Native style coverage on Qt 6.8, especially Linux (Fusion) and macOS without a Mac for agents (T-0011, D-0001).
- Gapless + device switching across backends (T-0003).
- macOS/Linux measurements deferred per D-0001.
