# ADR 0002 — Signal storage budget and tiered compaction

- **Status:** Accepted
- **Date:** 2026-10-29

## Context

ADR 0001 makes Tinwave collect a broad set of signals. Stored naively (UUID per column, one row per UI
impression, uncompacted, full context per event, per-track fine-grained context buckets), a heavy listener
would accumulate hundreds of MB per year, and the derived context buckets alone could reach gigabytes.
Large data hurts query speed, sync, backups, export and the "lightweight" pillar.

## Decision

- Hard budgets for the reference heavy user over 5 years: `signals.db` ≤ 60 MB, `taste.db` ≤ 80 MB.
- High-frequency UI signals (impressions, hover, dwell) are stored as daily counters, never raw events.
- Compact rows: integer surrogate keys and dictionaries in `signals.db`; `(device, seq)` event identity;
  device/timezone on the session row; quantized packed payloads; one secondary index.
- Tiered retention: raw events 90 days → play records 3 years → monthly rollups forever.
- Play records are written live at play end; aggregates and heatmaps are updated before compaction.
- Fine-grained context buckets only for artists/albums/genres; tracks use coarse buckets; pruning and caps.
- A budget enforcer shortens hot/warm windows if a store exceeds budget; user-explicit data is never touched.
- A 5-year synthetic growth test in CI enforces the budget.

## Consequences

- Taste can only be fully recomputed at the retained granularity; incremental aggregates carry older detail.
- Sync dedupe uses `(device uuid, seq)` instead of per-event UUIDs (still satisfies spec §8 "deduplicated by event ID").
- Every new signal must carry a size estimate and a compaction plan.

## Spec / docs updated

- `tinwave-specs.md` §10.1.
- `docs/rules/storage-budget.md` (new), `database.md`, `00-principles.md`, `rust.md`, `privacy-and-signals.md`,
  `qml-ui.md`, `testing.md`; `docs/guides/signal-catalog.md`, `taste-engine.md`, `audio-analysis.md`,
  `context-engine.md`, `adding-a-signal.md`; `CLAUDE.md`.
