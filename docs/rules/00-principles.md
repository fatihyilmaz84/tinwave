# 00 — Principles (non-negotiable)

Distilled from the spec plus the signal strategy. If a change violates one of these, it does not merge.

## Product

1. **Offline first.** Every core feature works with no network, no account, no server.
2. **The user's files are sacred.** Preview, journal, undo. No silent overwrite, no automatic delete.
3. **Sources are always visible.** Local file, streamed server track, downloaded server copy — never ambiguous.
4. **Lightweight.** Targets: idle 50–100 MB, browsing/playback 80–160 MB, scanning 150–300 MB (spec §12).
5. **Calm, native UI.** The OS draws the standard controls and chrome; Tinwave draws only music-specific parts,
   from tokens ([design system](../design/design-system.md)). Advanced controls exist; they do not crowd everyday use.

## Signals & recommendations

6. **Collect everything useful, keep it all local.** We are deliberately more observant than any
   streaming service: volume per section, speed changes, seeks, replays, skips, browsing dwell,
   time of day, daylight, weather, output device, activity context. The data never leaves the device
   (except the user's own device sync, and an opt-in coarse weather request).
7. **Instrument from day one.** Signal capture ships with the first build that can play audio.
8. **Raw events are append-only, then compacted.** Raw history is never edited; after 90 days it folds into
   per-play records and monthly rollups. Taste aggregates are updated before compaction, so nothing is lost.
9. **Small data.** Signal + taste data stays under ~150 MB after 5 years for a heavy listener
   ([storage-budget.md](storage-budget.md)). Count instead of log, coalesce, quantize, compact.
10. **Context explains, it does not accuse.** Volume down during a video call is context, not dislike.
    Every interpretation must consider context before scoring.
11. **Every recommendation is explainable.** Each pick carries at least one human-readable reason.
12. **The user sees everything.** The Signal Center shows every category, a live feed, storage use,
    export and wipe. No hidden collection. No dark patterns in consent.
13. **No creepy inputs.** Microphone, camera, screen contents, window titles, keystroke contents,
    global input hooks and browser history are banned forever (see privacy rules).
14. **Lightweight intelligence.** Statistics, bandits, similarity vectors. No resident ML model,
    no cloud AI (spec §10). Heavy work (audio analysis) runs only when idle and on AC power.

## Engineering

15. Rust domain code is independent of UI, OS, DB implementation and servers.
16. Bounded everything: queues, caches, concurrency, memory, **and disk**.
17. Everything cancellable and resumable: scans, analysis, compaction, sync, downloads.
18. Errors are explicit and user-meaningful. No silent fallbacks.
19. Decisions are written down (ADR) and the spec is updated when they change.
