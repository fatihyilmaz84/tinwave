HANDOFF T-0007 (round 1)
status: partial
branch: t/T-0007-storage-growth-test   commit: 5153e98 (on main ad05b6b)
summary: The storage growth harness runs 90 simulated days of the reference heavy listener in ~11–15 s through the real bus, two GestureCoalescers, UiCounters and SignalWriter into signals.db, with real codec-encoded SignalPayloads. 90-day hot window 4.38 MB (budget 6 MB). Every event group is 36–65 % of the catalog's bytes/day, but the catalog's context snapshot row is 2.09× low; that assertion fails and is committed #[ignore]d for the lead's decision. Everything else passes.

acceptance criteria:
- [x] Deterministic with a fixed seed — `the_same_seed_gives_the_same_sizes` (3 days twice: equal file size, per-table bytes, row counts, identical stored events; a different seed differs); the 90-day file is 4 378 624 B in every run. Writer kept in lock step with the driver (one tick = one play, UI counters before the play's events, play-end flushed at once, barrier until committed).
- [x] 90-day hot window ≤ 6 MB — `ninety_days_of_a_heavy_listener_fit_the_hot_window_and_the_five_year_budget`: 4.38 MB (48.7 KB/day); 0 dropped, 0 reserved overflows, 0 counter keys dropped, 0 failed commits, 0 events lost; < 60 s.
- [~] Per-group bytes/day within +50 % of the catalog — `ninety_days_match_the_catalog_volume_table_within_fifty_percent` passes for all six event groups and UI counters and checks generator rates within 25 %; `ninety_days_context_snapshots_match_the_catalog_estimate` fails (2.09×), committed `#[ignore = "fails: ..."]`, assertion strict.
- [x] < 60 s in release: 10.8–14.0 s (15 s debug); 90-day tests in the default run.
- [x] 60-step system-volume burst within 200 ms stores exactly one SystemVolumeChanged (P-0010) — `a_sixty_step_system_volume_burst_within_200_ms_stores_one_event` (real coalescer, bus, writer, DB; first from, last to; second burst 5 s later ⇒ 2 rows); the 90-day run also has bursts: 12 746 ticks ⇒ 981 stored events.
- [x] Real payloads encoded by the codec (P-0013): all 38 stored event kinds.

report (`cargo test -p tinwave-db --release --test storage_growth -- --nocapture`, seed 2026, 90 days, 10 779 plays = 120/day, 42 059 events = 467/day, 10 779 transactions, 409 571 counter interactions ⇒ 26 577 ui_counter rows):
- Per table (dbstat, incl. indexes): signal_event 1 569 KB; ui_counter 623 KB; context_snapshot 377 KB; dict_recording 868 KB; dict_source 553 KB; dict_entity 307 KB; session 29 KB; rest 4–8 KB each. File 4.38 MB, 104.1 B/event whole file.
- Per group vs catalog §9 (1 KB = 1000 B): play 239.5 ev/day, 8 940 B (14 000) 0.64; volume/mute/speed 56.1, 2 446 (4 000) 0.61; seeks/replays/previous/repeat/pause 56.9, 2 270 (4 000) 0.57; queue/fav/ratings/playlists/edits 39.1, 1 354 (2 500) 0.54; search 19.2, 688 (1 500) 0.46; output/EQ/window 30.0, 916 (2 000) 0.46; context snapshots 57.8, 4 187 (2 000) **2.09**; UI counters 295 rows, 6 918 (5 000) 1.38; not in §9 (views, sessions, radio feedback, explanations) 26.5, 817; dictionaries + sessions 19 615; total listed rows 28 535 (35 000).
- Events ~39 B/event (29–47 by kind) vs the catalog's assumed 58–67.
- Extrapolation: 48.7 KB/day; without compaction 5 years = 89 MB; with ADR 0002 tiering (90-day hot window + catalog 2.0 MB/year warm + 0.5 MB/year cold) 5 years = 16.9 MB (budget 60 MB).

gap vs the catalog (not loosened):
1. Context snapshots ~72 B per snapshot vs ~30 B (only the fields blob); real row adds id, 8-byte hash, key, cell overhead and the hash UNIQUE index (~53 B without the index). Proposal: catalog ~4.2 KB/day or drop the index; fix "~30 bytes each" in context-engine.md.
2. Dictionaries missing from §9: 19.6 KB/day over 90 days (1.77 MB, 40 % of the file), saturating as the library is used. Proposal: add a row.
3. UI counters 6.9 KB/day vs 5 KB (1.38×): update to ~7 KB/day.
4. Event groups 36–65 % of the catalog; could tighten to ~39 B/event.

files changed: crates/tinwave-db/tests/storage_growth.rs, tests/growth/mod.rs, tests/growth/sim.rs. No Cargo, benches or production code.
checks: fmt pass, clippy pass, tests 332 passed / 0 failed / 3 ignored (one is the context check), deny pass; release file run 4 passed, 1 ignored, 3 times.
signals: none.
assumptions: bytes per group computed from the SQLite record format and scaled to real pages (×1.146 fill); Paused/Resumed in the §3 group; kinds not in §9 reported separately; context_snapshot rows inserted directly (no context hub yet: id, 64-bit hash, u32 key, 30-byte fields), ~every 8 events; `PlaylistEdit.playlist_id` stand-in small id; WAL checkpoint thresholds raised (96 MiB soft) in the harness (file byte-identical either way); compaction hook `growth::run(cfg, on_day_end)` at a per-day barrier.
proposals:
- P: Add a "Dictionaries and sessions" row and measured context-snapshot and UI-counter costs to catalog §9 | size: S
- P: Decide whether `context_snapshot.hash` stays UNIQUE (index ~20 B of 72 B; dedupe could be an in-memory map rebuilt at startup) | size: S
questions for lead:
- OK to land with the context check #[ignore]d until the catalog/schema decision, or leave it enabled and red?

---
Lead answers (2026-11-02): the catalog was wrong, not the design — §9 updated (context ~4.5 KB/day, UI counters ~7 KB/day,
dictionaries row, measured total), context-engine.md corrected; `hash UNIQUE` stays (P-0085 rejected). Un-ignore the
context check against the corrected catalog, and add a dictionaries assertion against the new row, before review.
