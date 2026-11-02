---
id: E-01
title: Desktop library & playback, instrumented from day one
status: planned
spec: §16 Phase 1, §6, §7, ADR 0001, ADR 0002
---

## Outcome
The PO can import a music folder, browse and search it, play music with a persistent queue, favorite and build
playlists — and open the Signal Center to watch every interaction being recorded locally, compactly, and wipeable.

## PO acceptance demo
1. Import a 20k+ track folder; browse artists/albums/tracks while the scan runs; search is instant.
2. Play, seek, skip, change volume (app and keyboard keys), change output device; quit and resume the queue.
3. Signal Center: live feed shows each action with position, section-less for now, input source and context;
   storage panel shows sizes vs budget; export and wipe work.

## Tickets (to be groomed by the lead after E-00 spikes land)
- library.db schema + migrations; scanner (incremental, cancellable) + tag reading
- FTS5 search
- Playback engine on the chosen backend; queue persistence; gapless. In-app volume uses session volume or software gain,
  never endpoint volume, and passes Tinwave's event-context GUID if it ever touches endpoint volume (ADR 0004, P-0009)
- Play records written live at play end
- T0 playback signals (start/complete/skip/seek/replay/pause/volume/mute/queue/favorite)
- ContextHub (snapshot assembly): exclude or bucket the 1 % `system_volume` field from the snapshot hash, or snapshots
  explode past the ~60/day target (T-0014 review note).
- T1 context providers: clock, power, idle, output device, system volume (Windows first); output device + system volume
  on Windows is T-0014 (ADR 0004)
- `Theme.qml` tokens + style selection (from T-0011), then the `Tw*` component catalog (design-system.md §5) as
  small design tickets before the screens that use them
- Install a `tracing` subscriber in the app composition root (the signals writer logs failed commits at warn/error;
  P-0072).
- Desktop shell (CXX-Qt) per the design canvas: Library, Album, Artist, Now Playing, Queue views. The ticket that
  creates `apps/tinwave-desktop` must add `"apps/*"` to workspace `members` (T-0001 left it out: Cargo rejects an
  empty glob)
- OS media integration: SMTC first (D-0001), then MPNowPlaying and MPRIS
- UI counters (dwell/hover/impressions) through `SignalEmitter`
- Signal Center screen with storage panel, export, wipe — list and JSONL export driven by `SignalKind::payload_fields()` (P-0012); diagnostics list includes audio provider `callback_dropped`/`resyncs` (P-0077)
- Compaction job (hot → warm → cold) + incremental vacuum

## Design
All screens follow docs/design/design-system.md (ADR 0003). Canvas references: "Library · Windows",
"Album + Queue · macOS", "Signal Center · Windows dark". The PO demo is run in light and dark.

## Risks & open questions
- Depends on E-00 bridge and audio backend ADRs, and on T-0011 for the style/Theme approach.
