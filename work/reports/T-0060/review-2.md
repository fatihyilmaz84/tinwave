REVIEW T-0060 (round 2)
verdict: APPROVE
checks (run by reviewer): fmt pass (exit 0), clippy pass (`-D warnings`, exit 0), tests pass (nextest "665 tests run: 665 passed (4 slow), 6 skipped", exit 0, worktree @ 6264ede), deny pass ("advisories ok, bans ok, licenses ok, sources ok"). Merge with current main 905e2c6: `git merge-tree --write-tree main t/T-0060-wipe-masks-context-key` is clean (tree 628dbc7). I checked that tree in a git-archive copy with its own CARGO_TARGET_DIR. fmt was clean except my own probe file, deny ok, and `cargo nextest run -p tinwave-db -p tinwave-signals` gave "250 tests run: 250 passed" (248 from the branch plus my 2 probes). main has no crates/ or Cargo.lock change since the branch base.

acceptance criteria:
- [x] Wiping each T2/T3 context category zeroes exactly its ContextKey fields in every play row, other fields untouched — verified by: `wiping_a_category_clears_exactly_its_fields_in_every_play_row`, `wiping_the_place_or_the_clock_also_clears_the_derived_activity` and `wiping_a_tier_clears_the_fields_of_its_categories`. The new `many_distinct_keys_are_rewritten_exactly_as_clear_raw_does` compares both tables row by row with `clear_raw` for every category and tier, and checks the report counts. All pass.
- [x] A key whose last known field is wiped becomes raw 0 — verified by: `a_key_whose_last_field_is_wiped_becomes_raw_zero` and `clearing_the_last_known_field_gives_raw_zero`. The SQL CASE returns 0 when `(c & keep) & 0xFFFFFF = 0`, which is the same rule as `clear_raw`.
- [x] The wipe is atomic — verified by: the three injection tests still pass against the new SQL. The snapshot trigger fires after the play UPDATE; the play trigger fires on `OLD.context_key = a`, which the single UPDATE reaches after it has already changed other rows; the stray key sits in context_snapshot, which is checked after plays are rewritten. The out-of-range check runs inside the same IMMEDIATE transaction (wipe.rs `rewrite_keys`) and returns `OutOfRange`, so the tx is dropped and rolled back.
- [x] fmt, clippy, nextest, deny pass; the storage growth test still passes — verified by: the runs above (the 4 storage_growth ninety_days tests passed, slow as before).

Round-1 blocking finding (per-key scans under the write lock), resolved:
- **SQL against `clear_raw`, by reasoning.** `keep = 0xFFFFFFFF ^ fields.bits()` and `region = ContextFieldSet::ALL.bits() = 0x00FFFFFF`. Inputs are proven to be in 0..=u32::MAX before the UPDATE, so `c & keep` equals `raw & !bits`, and the CASE is `clear_raw` exactly.
  - The WHERE `c != 0 AND ((c & clear) != 0 OR (c & region) = 0)` is exactly "the new value differs from c". If clear bits are set, the value changes. If not, `c & keep = c` and it changes only when it becomes 0, which happens only when the field region is empty.
  - The builder's extra `OR (c & region) = 0` is therefore necessary. Without it, version-only or reserved-only keys (e.g. `1<<28`, `(1<<28)|(1<<24)`, `0xF0000000`) would stay non-zero, while `clear_raw` makes them 0. My suggested WHERE in round 1 was wrong on this point.
- **SQL against `clear_raw`, by experiment.** My scratch test ran the exact CASE/WHERE in SQLite over 30,015 values and all 128 field subsets, comparing against `ContextKey::clear_raw`. Values covered: 0, 1, 3, version-only, reserved bits, other versions, 0x7FFFFFFF, 0x80000000, u32::MAX-1, u32::MAX, 0xDEADBEEF, 20k random u32, 5k high-byte-only and 5k version-1 random field values. No mismatch, in either the SET value or the WHERE row selection.
- **Out-of-range values.** Negative, > u32::MAX and non-integer values (`typeof != 'integer'`) are caught by the check scan and roll back. `a_stored_key_that_is_not_a_u32_fails_the_wipe_and_changes_nothing` covers the negative case. NULL cannot occur (NOT NULL).
- **Lock time.** In a scratch debug build with 131,400 plays and 4,000 distinct keys, a full read scan took 14 ms and the NetworkPlace wipe took 107 ms (131,239 rows rewritten). A concurrent `BEGIN IMMEDIATE` with a 5 s busy timeout succeeded after 102 ms. That is one check scan plus one rewrite scan, against 16.3 s / "database is locked" in round 1. With 12 copies in parallel, the wipe took 100-137 ms and every concurrent writer got the lock within 4-697 ms; the high end is SQLite busy-handler backoff.
- **Timing test is meaningful and not flaky.** The builder reports that the reverted per-key loop fails it at 5.57 s. I ran `a_write_batch_during_a_wipe_of_the_reference_table_is_not_starved` 12 copies at once, twice: 24/24 passed, each finishing in 8.5-11.7 s wall time, most of it fixture setup. The 5 s budget is about 40x the measured lock hold even under that load.

blocking findings:
- none

design check: n/a (no UI)

non-blocking findings:
- crates/tinwave-db/tests/context_wipe.rs (the `not_starved` test, probe loop) — If the wipe finishes before the probe sees its lock, the batch runs uncontended and the test still passes. It cannot give a false failure, and it does catch the regression, but on a very fast machine it may stop exercising contention. It could assert that the probe observed the lock at least once, or record whether it did.

proposals:
- P: Wire the Signal Center wipe to `SignalWiper`, off the GUI thread | why: nothing calls it yet; a wipe is about 0.1-0.2 s at reference size | size: S
- P: The context hub writes snapshot rows only inside the writer transaction and handles hash/key divergence after a wipe (now documented in wipe.rs "Not yet covered") | why: a post-wipe insert built from pre-wipe values would reintroduce wiped fields | size: S

notes for lead: The merge with main is clean. All my scratch files (rv60r2-* in C:\Users\FSYY\AppData\Local\Temp\claude\D--dev-Tinwave\53bf9027-c019-4dd4-82f1-5a72c6705b0c\scratchpad) were listed and then deleted. Nothing I started is still running, and the worktree is untouched (clean, HEAD 6264ede). The wipe API is still uncalled (the round-1 proposal stands). The `context_snapshot.fields` blob and the hash wipe stay parked with the context-hub/encoding ticket, as you ruled.
