REVIEW T-0007 (round 1)
verdict: CHANGES_REQUESTED
checks (run by reviewer at 6275672): fmt pass, clippy pass, tests 334 passed / 0 failed / 2 ignored (neither in this ticket), deny pass. Release storage_growth: 6 passed in 15.9 s (14.8 s waiting for the writer).

acceptance criteria:
- [ ] Deterministic with a fixed seed — depends on thread scheduling (blocking 1). Normal runs hold (36/36 of `the_same_seed_gives_the_same_sizes` with 12 parallel copies × 3 rounds), but with the producer preempted right after a `Paused`, two same-seed runs store different ids.
- [x] 90-day hot window ≤ 6 MB — 4 378 624 B; 0 dropped/overflows/keys dropped/failed commits/lost.
- [x] Per-group bytes/day within +50 % — event groups 0.46–0.64, context 0.93, UI counters 0.99, dictionaries + sessions 0.98; generator rates within ±25 %.
- [x] < 60 s in release — 15.9 s; `elapsed < 60 s` asserted, held in the debug workspace run.
- [x] 60-step volume burst ⇒ one SystemVolumeChanged (real coalescer, bus, writer, DB; first from / last to; gesture 150–200 ms; second burst ⇒ second row); 90-day run: 12 746 ticks ⇒ 981 events.

blocking findings:
1. tests/growth/sim.rs:10-18 and :386-409 — the lock step is not scheduling-independent. `Paused` (emitted mid-play by PauseResume) also ends a play (`ends_a_play`, accepted in T-0006); the writer commits as soon as it pulls `Paused`, and how many following events of the same play it has pulled depends on timing. A store transaction resolves event dictionary ids before counter ids, so a new recording after `Paused` gets a different id relative to the tick's new counter entities depending on the batch cut (`signal_event.recording` and `ui_counter.entity` change). Evidence: a 3 ms sleep after `Paused` in ~half the runs (stand-in for CI preemption): two same-seed 30-day runs committed 3 620 vs 3 630 batches, 4 event rows with different recording ids, different ui_counter rows (second pair: 1 differing event row). File size happened to be equal. `the_same_seed_gives_the_same_sizes` compares stored rows, so it can flake. Fix: in `Sim::emit`, `self.barrier()` after any event where `tinwave_db::ends_a_play(kind)` is true (verified in scratch under the same preemption: 3 pairs, identical 3 771 batches, 0 differing rows, identical ui_counter); correct the doc.

design check: n/a

non-blocking findings:
- storage_growth.rs:269-273 — `.replace("<newline>", "<newline>")` is a no-op (intended `"\r\n"` written as a literal newline); same for `"\n"` in assert messages at :217-221, :240-244, :257-261 (harmless). Drift guard still works because `str::lines()` strips CR. Use escapes.
- storage_growth.rs:268-300 — the drift guard is real (reviewer edits: context ~4.5→~5 KB FAIL; play ~14→~12 KB FAIL; UI counters ~7→~5 KB FAIL; no change pass), but it checks hard-coded strings, not the `CATALOG_*` constants / `Group::catalog()`; editing a constant alone wouldn't trip it. Build expected strings from the constants.
- growth/mod.rs:348-353 — the 5-year extrapolation treats the dictionary tables (1.77 MB at 90 days) as part of the hot window; compaction never deletes them and they grow until the 50k library is covered (a few more MB, far under 60 MB). Say so next to the warm/cold assumption (which is labelled at mod.rs:36-40 and in the report).
- Attribution sound: record-format cell sizes scaled by table pages / sum of estimates (×1.146); groups sum to the table ((8940 + 2446 + 2270 + 1354 + 688 + 916 + 817) × 90 ≈ 1 568 790 B vs signal_event 1 568 768 B).
- Shortcuts minor: context_snapshot rows inserted directly (conservative random 64-bit hash, u32 key, 30 random field bytes); `PlaylistEdit.playlist_id` stand-in 1..40 understates ~3 events/day by 1 byte. Everything else through the real bus, both coalescers, UiCounters, SignalWriter, codec payloads, ctx on every event (57.8 changes/day); 120 plays × ~228 s ≈ 7.6 h/day.

proposals:
- P: Update testing.md "Storage growth test" (still names tinwave-signals/benches/growth.rs as the generator location) | size: S

notes for lead:
- Finding 1 is cheap (one barrier per play-end event) but blocking: determinism is an AC and the 5-year compaction run builds on this harness. Couldn't make it flake under normal load; needed forced preemption.
- Scratch: session scratchpad rev-t0007-r1/ (REV_PREEMPT / REV_FIX switches, reviewer_split_probe), t7det/ logs; deletable.

---
Lead (2026-11-02): round 2 sent with blocking 1 plus all non-blocking items; testing.md location fixed by the lead (P-0086).
