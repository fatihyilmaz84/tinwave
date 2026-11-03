REVIEW T-0074 (round 1)
verdict: APPROVE
checks (run by reviewer): fmt pass (exit 0); clippy pass (`--workspace --all-targets -D warnings`, Qt env, exit 0; also lib-only `cargo clippy -p tinwave-db -- -D warnings` exit 0, so the gated `Connection` import is not flagged as unused without the feature); tests pass (`cargo nextest run --workspace --no-fail-fast`: "Summary [88.844s] 814 tests run: 814 passed (4 slow), 7 skipped", exit 0); deny pass ("advisories ok, bans ok, licenses ok, sources ok", exit 0)

acceptance criteria:
- [x] A default-feature build cannot reach a raw Connection through tinwave-db. Verified by:
  - `cargo tree --workspace -e normal,build,features -i tinwave-db` shows only `feature "default"`.
  - `-e all,features` shows `test-support` only under tinwave-db's own `[dev-dependencies]` self edge.
  - No crate depends on tinwave-db today, desktop included: `cargo tree -p tinwave-desktop -i tinwave-db` gives "did not match any packages". So a desktop build compiles no tinwave-db code at all.
  - Consumer test: in a scratch copy I added a workspace crate with a default-feature path dependency that calls `connection_for_tests()` on both `SignalWiper` and `SignalStore`. `cargo check -p zz-consumer` gives two E0599 errors ("no method named `connection_for_tests`"). `cargo check -p zz-consumer -p tinwave-db` (no test targets) gives the same errors.
- [x] write_batch statement count is pinned and a per-row mutation fails it. Verified by:
  - Tests `signal_store::a_batch_runs_a_statement_count_set_by_the_batch_whatever_is_already_stored` and `an_empty_batch_runs_no_statement`.
  - The doc comment explains the formulas (crates/tinwave-db/tests/signal_store.rs, around lines 1138-1155):
    - `6 + 8n` for n plays = BEGIN/epoch/COMMIT 3 + device/session/source lookups 3 + per play (recording lookup+insert 2, event inserts 2, play row 4).
    - `3 + 3n` for n counter rows = 3 + per row (lookup, insert, upsert).
    - 36 for a replayed batch of 10 plays = 3 + 13 lookups + 20 ignored inserts.
  - Each case runs on a fresh store (cold cache) over a copy of a 3-play database and of a 3000-play database. The test requires the same count on both, the exact pinned count, first statement `BEGIN IMMEDIATE` and last `COMMIT`.
  - Mutations I ran myself (scratch copy, own CARGO_TARGET_DIR, recompile seen each time):
    - (a) One `SELECT ?1` per stored play row inside the counter-upsert loop: FAILS. "1 counter rows: 10 on the small database, 3007 on the large one / expected 6"; "10 counter rows: 73 vs 30043 / expected 33".
    - (b) The transaction left open (`DropBehavior::Ignore`, no COMMIT) for the 1-play batch: FAILS. "1 plays: 13 statements, expected 14" and "1 plays: not one transaction".
    - With the leak applied to every play batch, the replay assertion `events_duplicate == 20` also fails.
- [x] fmt, clippy, nextest, deny pass. See the checks line.
- [x] (Scope, P-0301) Fixtures are cheap and BEGIN/COMMIT/checkpoint statements are still counted.
  - `populate()` now runs in one transaction.
  - Templates are built once per test and copied per case with `TempDb::copy`. It runs a TRUNCATE checkpoint and asserts busy == 0, then copies into a new tempdir.
  - `statements_of` still traces every statement on the wiper connection.
  - I could not confirm the builder's "about 2 s alone": the machine was at 100% CPU from other agents. The unmutated signal-wipe count test took 30.4 s alone in my scratch copy and 30.7 s in the full run (it was 84.6 s of an 89 s suite in T-0073's review). The structural change is evident.

Points the lead asked about:
1. Feature design.
   - Plain builds (`cargo build/check -p X`, `cargo build --workspace` without `--all-targets`) never enable `test-support`.
   - Resolver-2 caveat, which I reproduced: dev-dependency features are unified whenever tinwave-db's test targets are part of the same build. `cargo check -p zz-consumer -p tinwave-db --all-targets` compiled the forbidden call (exit 0).
   - So `cargo clippy --workspace --all-targets` and `cargo test`/`nextest --workspace` would not catch a future misuse. Only a plain build would. `cargo build --workspace` is in the CLAUDE.md command list, and it catches it. Not blocking, because no crate depends on tinwave-db today. See proposals.
2. Pinned formulas: explained, they catch per-row and leaked-lock mutations (above), and BEGIN IMMEDIATE / COMMIT are part of the count and checked explicitly.
3. T-0073 guarantees hold.
   - The diff changes no expected length in signal_wipe.rs or context_wipe.rs, only fixtures and failure-message formatting.
   - My mutation: a leaked `BEGIN IMMEDIATE` after `checkpoint_best_effort()` in the signal tier wipe (crates/tinwave-db/src/wipe.rs:293). `a_wipe_runs_a_fixed_number_of_statements_whatever_the_row_count` FAILS: "signal tier T0: 9 statements, expected 8", T1 9/8, T2 5/4, T3 5/4.
4. The busy-timeout test fix (crates/tinwave-db/tests/signal_wipe.rs, `a_batch_that_outwaits_the_busy_timeout_is_stored_by_the_retry`) is not weaker.
   - It still asserts `attempts >= 2`, `waited >= BUSY_TIMEOUT`, `after_wipe <= AFTER_WIPE_SLACK` and the row stored.
   - The holder now releases only after the helper has seen a real busy give-up. That proves the first attempt hit the timeout, where before this was inferred from wall time.
   - A zero or short busy timeout would still fail on `waited >= BUSY_TIMEOUT`.
   - If the first attempt never gives up, the holder panics after LOAD_BUDGET (120 s). Unwinding drops the holder connection and releases the lock, so the test fails instead of hanging.
   - It passed in my full run (6.4 s).
5. Fixture copying is safe.
   - Templates are local to each test, never statics.
   - Every copy goes to its own `tempfile::tempdir()` and is removed on drop, unwind included.
   - The template is not written to while it is copied.
6. Cargo.lock: the only change is `+ "tinwave-db",` in tinwave-db's own dependency list (the self edge).

blocking findings:
- none

design check: n/a

non-blocking findings:
- crates/tinwave-db/tests/common/mod.rs:562 — the doc comment holds a literal backspace byte (0x08): "counts in \x08usy". It was meant to be "in `busy`", probably mangled by a shell heredoc. Replace it with the intended text.
- crates/tinwave-db/Cargo.toml:11 — the feature comment names only `SignalWiper::connection_for_tests`. `SignalStore::connection_for_tests` (crates/tinwave-db/src/store.rs:431-439) is gated by the same feature.
- crates/tinwave-db/tests/signal_store.rs:1252 — the replayed-batch case pins 36 but skips the first-`BEGIN IMMEDIATE` / last-`COMMIT` check the other cases have. The exact count still catches a leaked lock.
- crates/tinwave-db/tests/signal_store.rs (`with_stored_plays`) — the large template only has more plays, events and recordings. `ui_counter`, `dict_entity` and `session` hold the same rows in both templates, so the small-vs-large comparison proves nothing for the counter path or for per-session work. The exact pin still catches any added statement.
- .config/nextest.toml:5-6 — the comment "T-0074 makes it cheaper" is now stale (the builder raised this as a proposal; outside scope).

proposals:
- P: Add a non-test `cargo check --workspace` (lib/bin only) to the gate list | why: under resolver 2, `clippy --all-targets` and `nextest --workspace` turn on `test-support` for every consumer, so only a plain build catches a future product call to `connection_for_tests` | size: S
- P: Grow ui_counter, dict_entity and session rows in the large write_batch template (3 vs 3000) | why: makes "independent of rows already stored" hold for the counter and session paths too, not just plays/events/recordings | size: S
- P: Fix the 0x08 byte and the feature comment (the two non-blocking items above) | why: a control character in source, and a doc comment that no longer matches the code | size: S

notes for lead:
- No merge-order concerns: the diff stays in crates/tinwave-db plus one Cargo.lock line, and nothing under apps/.
- I could not reproduce the timing improvement on this machine (100% CPU from other agents).
- Cleanup: my scratch copy, its target dir and my nextest log under the session scratchpad are deleted. None of my processes are running. A python.exe started 2026-11-02 and a cargo process belong to other sessions.
- Mutation hygiene: restoring a file with Copy-Item keeps its old mtime, so cargo silently reuses the mutated binary. I hit this and fixed it by touching the restored files and checking that the recompile ran. Builders should do the same.
