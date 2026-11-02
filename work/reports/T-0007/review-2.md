REVIEW T-0007 (round 2)
verdict: APPROVE
checks (run by reviewer at 0f3f2e7): fmt pass, clippy pass, tests 335 passed / 0 failed / 2 ignored (not this ticket), deny pass. Release storage_growth: 7 passed in 13.8 s (15.4 s wall). Numbers byte-identical to round 1 (file 4 378 624 B; context 0.93; counters 0.99; dictionaries + sessions 0.98; 5-year 16.9 MB).

acceptance criteria:
- [x] Deterministic with a fixed seed — `the_same_seed_gives_the_same_sizes` and new `the_rows_are_the_same_when_the_producer_is_preempted_after_a_pause` compare batch count, file size, per-table bytes, event rows and ui_counter rows incl. dictionary ids. Reviewer's round-1 random-preemption repro ported (3 ms stall after ~half of all Paused, chosen by wall clock; 30 days, two runs): 3/3 identical (3 771 vs 3 771 batches). Mutation keeping the stall but skipping the barrier for Paused: new test fails 3/3 (590 vs 604). 10 parallel copies of both determinism tests: 20/20 ok.
- [x] 90-day hot window ≤ 6 MB (4.38 MB; nothing dropped, failed, lost).
- [x] Per-group within +50 % with a readable report.
- [x] < 60 s in release (13.8 s; asserted).
- [x] 60-step burst ⇒ one SystemVolumeChanged.

round-1 items: blocking 1 fixed (sim.rs:419-425 barrier after every ends_a_play event; doc sim.rs:10-23 correct; test-only preempt_after_pause off by default, not from env). Drift guard tied to both sides (CATALOG_COUNTERS 7.0→7.5 FAIL; Group::catalog() Search 1.5→2.0 FAIL; catalog Queue ~2.5→~3 FAIL; no change pass). Escapes fixed. Extrapolation states dictionaries aren't compacted (mod.rs:36-40, :365-368, report line). Scope clean (tests/storage_growth.rs, growth/mod.rs, growth/sim.rs; no TODOs, no #[ignore]).

blocking findings:
- none

design check: n/a

non-blocking findings:
- none new (round-1 minor notes stand: direct context_snapshot inserts until the context hub exists; small playlist_id stand-in bias negligible).

proposals: none

notes for lead: nothing blocks the merge. Scratch: rev-t0007-r2/ (+ backups), t7det2/ (deletable).
