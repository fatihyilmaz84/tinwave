# ADR 0001 — On-device signal pipeline from Phase 1

- **Status:** Accepted
- **Date:** 2026-10-29

## Context

Tinwave's main differentiator for discovery is recommendations that understand the listener deeply:
which sections they turn up, what they skip, how they listen at different times, in different weather,
on different devices, during different activities. The spec places Smart Radio in Phase 7, but listening
history cannot be reconstructed later. The spec also requires local-first data, no telemetry by default,
and no resident ML model (§1, §10, §12, §15).

## Decision

- Ship a signal pipeline (`tinwave-signals`, `tinwave-context`) with the first playable build (Phase 1).
- Capture a broad catalog of interaction and context signals ([signal catalog](../guides/signal-catalog.md)),
  stored append-only in a separate `signals.db`.
- All signals stay on-device; only the opt-in weather provider sends rounded coordinates off-device.
- Sensitive local signals (foreground-app category, network place, calendar) are opt-in.
- A Signal Center UI with live feed, export and wipe ships with the pipeline.
- Recommendation modeling uses decayed aggregates, context-bucketed Beta posteriors, Thompson sampling
  and similarity vectors — no resident ML model.

## Consequences

- Phase 1 scope grows by the signal bus, writer, Signal Center and T0/T1 providers.
- Every playback/UI feature must emit its signals (enforced in review and tests).
- Smart Radio in Phase 7 starts with months of real history.
- Privacy posture remains compatible with spec §15 and store/regulatory review.

## Spec / docs updated

- `tinwave-specs.md` §10 and Phase 1.
- `CLAUDE.md`, `docs/rules/*`, `docs/guides/*`.
