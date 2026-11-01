# Storage budget

We collect a lot, but it must never become gigabytes. A heavy listener with a big library should
stay **under ~150 MB of signal + taste data after 5 years**, and a typical user well under 50 MB.
Size is a feature: small data is fast to query, fast to sync, fast to back up, and cheap to export.

## Budgets

Reference user: **heavy listener** — 8 h/day, ~120 tracks/day, 50 000-track library, 5 years of history.

| Store | Budget (reference user, 5 y) | Typical user | Enforced by |
|---|---|---|---|
| `signals.db` | ≤ 60 MB | ≤ 15 MB | Tiered retention + budget enforcer |
| `taste.db` | ≤ 80 MB | ≤ 20 MB | Caps + pruning (rebuildable) |
| Context snapshots | inside `signals.db` | — | Dedupe + quantization |
| Logs | ≤ 20 MB | — | Rotating files, 4 × 5 MB |
| Artwork cache | user setting, default 500 MB | — | LRU (not signal data) |

The budget is configurable in Settings → Signal Center → Storage. The defaults above are hard targets
verified by the growth test (see [testing.md](testing.md)).

## Where the bytes go — and the rules that keep them small

### 1. Don't store what you can count

High-frequency, low-value UI signals are **never stored as raw events**. They are aggregated in memory and
flushed as counters:

| Signal | Stored as |
|---|---|
| `Impression` | `(day, entity, count)` counter, flushed with the batch writer |
| `ArtworkHover` | `(day, entity, count, total_ms)` counter |
| `PageDwell` | `(day, entity, count, total_ms, max_scrolled_fraction)` counter |
| `SignalDropped` | in-memory diagnostics only |

Rule: **a signal expected to fire more than ~50 times per hour is a counter, not an event.**

### 2. Coalesce gestures

Already required by [qml-ui.md](qml-ui.md): one event per volume/speed gesture, not per slider tick.
System-volume callbacks are coalesced over 600 ms. Repeated seeks within 2 s merge into one seek
(first `from`, last `to`).

### 3. Compact rows

- Integer surrogate keys **inside signals.db**: `recording`, `source`, `session`, `device` are `INTEGER`
  references to small dictionary tables. A 16-byte UUID appears once in the dictionary, not on every event.
- Event identity for sync dedupe is `(device, seq)` — device-local monotonic sequence — not a per-event UUID.
- Device id and timezone offset live on the **session** row, not on each event.
- `tier` is derived from `kind`, not stored.
- Payloads are tiny fixed binary structs (CBOR with integer keys or a hand-packed layout), with
  quantized values: volume as `u8` (0–255), rate as `u8` (hundredths − 50), fractions as `u8` (per-255).
- SQLite varints mean small integers cost 1–3 bytes. Prefer small integers over floats and blobs.
- One secondary index on events (`recording, ts`). Everything else uses rowid ranges (rowid ≈ time order).

Target: **≤ 60 bytes per raw event on disk including index.**

### 4. Tiered retention (hot → warm → cold)

| Tier | Granularity | Default kept | Approx. size (reference user) |
|---|---|---|---|
| **Hot** | Raw events + context snapshots | 90 days | ~6 MB |
| **Warm** | One **play record** per play (see below) | 3 years | ~2 MB / year |
| **Cold** | Monthly rollup per recording | Forever | ~0.5 MB / year |

A **play record** compresses everything about one play into one row (~40–50 bytes):

```
play(id, session, recording, source, started_ts, listened_ds (deciseconds), end_reason (u8),
     end_fraction (u8), flags (u16 bitset: favorited, replayed, seek_back, seek_fwd_intro,
     vol_up_early, vol_down, rate_up, rate_down, lyrics, queued, removed_after…),
     peak_section (u8), context_key (u32 packed))
```

`context_key` is the packed taste-engine bucket (daypart, weekday type, weather bucket, device class,
activity, place) — 32 bits instead of a full snapshot.

The **monthly rollup** keeps `(month, recording, plays, completes, skips_early, skips_late, favorites_delta,
listened_s, score_sum)`.

Compaction (raw → play records → monthly) runs when idle, in small transactions, oldest first.
Section heatmaps and decayed aggregates are updated **before** raw events are compacted, so nothing the
taste engine needs is lost.

### 5. Budget enforcer

If a store exceeds its budget anyway (e.g. extreme usage or a long retention setting):
1. Shorten the hot window (down to a 14-day floor).
2. Then shorten the warm window (down to a 1-year floor).
3. Never touch cold rollups or user-explicit data (favorites, ratings, playlists, "Never in this context").
4. Show a notice in the Signal Center explaining what was compacted.

### 6. Bounded derived data (taste.db)

| Data | Cap / encoding |
|---|---|
| Context Beta buckets | Track level: only coarse key (daypart × weekday type). Full key only for artists and genres. Prune buckets with decayed evidence < 0.5. Max 16 buckets per entity (keep highest evidence). |
| Decayed aggregates | 3 × `f32` + last-update `u32` per entity with any signal — untouched tracks have no row. |
| Section heatmaps | Only tracks with ≥ 3 relevant events. 64 bins × `u8`, sparse-encoded; decay + renormalize so values never overflow. |
| Similarity neighbors | Top 20 per track: `u32` id + `u8` quantized similarity (5 bytes each). |
| Co-play PMI | Top 50 per track, pruned monthly. |
| Audio features | Fixed ~64-byte struct per track; energy curve as `u8` per 2 s (≈120 bytes for a 4-min track); sections ≤ 16 × 4 bytes. |
| Explanation caches | Not persisted — regenerated on demand. |

### 7. SQLite hygiene

- `auto_vacuum = INCREMENTAL`; run `PRAGMA incremental_vacuum` after compaction, when idle.
- `PRAGMA optimize` on close. `wal_autocheckpoint` default; cap WAL with `journal_size_limit = 16 MB`.
- Page size 4096.

### 8. Export

Export is compressed (`.jsonl.zst`). Raw events export as-is; older periods export as play records and rollups.

## Rules for new signals

Every new signal must state an **expected bytes/day** for the reference user in
[signal-catalog.md](../guides/signal-catalog.md). Anything over **20 KB/day** needs a counter, coalescing,
or an ADR.
