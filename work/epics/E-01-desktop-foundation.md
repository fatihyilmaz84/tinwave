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
- Playback engine on the chosen backend; queue persistence; gapless
- Play records written live at play end
- T0 playback signals (start/complete/skip/seek/replay/pause/volume/mute/queue/favorite)
- T1 context providers: clock, power, idle, output device, system volume (Windows first)
- `Theme.qml` tokens + style selection (from T-0011), then the `Tw*` component catalog (design-system.md §5) as
  small design tickets before the screens that use them
- Desktop shell (CXX-Qt) per the design canvas: Library, Album, Artist, Now Playing, Queue views
- OS media integration: SMTC first (D-0001), then MPNowPlaying and MPRIS
- UI counters (dwell/hover/impressions) through `SignalEmitter`
- Signal Center screen with storage panel, export, wipe
- Compaction job (hot → warm → cold) + incremental vacuum

## Design
All screens follow docs/design/design-system.md (ADR 0003). Canvas references: "Library · Windows",
"Album + Queue · macOS", "Signal Center · Windows dark". The PO demo is run in light and dark.

## Risks & open questions
- Depends on E-00 bridge and audio backend ADRs, and on T-0011 for the style/Theme approach.
