REVIEW T-0032 (round 1)
verdict: APPROVE
checks (run by reviewer): fmt pass (`cargo fmt --all --check`, exit 0), clippy pass (`cargo clippy --workspace --all-targets -- -D warnings`, exit 0, Qt env set), tests pass (`cargo test --workspace`, exit 0, 0 failed; the ignored counts (1/4/2) were already on main, and this diff adds no ignored tests), deny pass (advisories ok, bans ok, licenses ok, sources ok)

acceptance criteria:
- [x] Item 1: the paused banner clears in the same session after the first successful save. Verified by `the_paused_banner_clears_after_the_first_successful_save_only` (crates/tinwave-db/tests/signal_settings.rs:435). A trigger-injected failed save keeps the banner and leaves the row absent, the first good save clears it, a later save keeps it clear, and `load()` still reports `FailedClosed(MissingRow)`. Also verified by `restored_settings_never_report_a_problem_before_or_after_a_save`, plus in-session asserts added to the missing-row and corrupt-row tests (lines 251, 314). Code: settings.rs:238-245 clears `problem` only after `execute` succeeds, because the `?` returns first.
- [x] Item 2: the documented alternative, which the ticket allows. The module doc in settings.rs ("Toggle only through here") says what goes wrong and why; `PersistentSettings::settings()` (settings.rs:178-183) says not to call the setters and links to the right ones; `SignalSettings::set_category`/`set_tier` (crates/tinwave-signals/src/settings.rs:207-221) say "in memory only", name `tinwave_db::PersistentSettings::set_*` as the app route, and say why the methods stay public (plain backticks, correctly not intra-doc links). The remaining gap (the same `Arc` reachable via `SignalBus::settings()`) is covered by the method docs and parked as P-0126.
- [x] Item 3: a deterministic test fails when the mask is read before the lock. Verified by `settings::tests::the_mask_is_read_under_the_connection_lock` (settings.rs:306). Mutation run by the reviewer on a scratch copy: FAILS with `left: 894, right: 1918` at settings.rs:336; passes on the real code. `the_hook_runs_once_and_only_for_the_next_save` checks the hook fires once.
- [x] fmt, clippy `-D warnings`, tests, `cargo deny check` pass.

Lead's three checks:
- The `#[cfg(test)]` hook costs nothing outside tests: field (107-108), initializer (144-145) and the block in `lock_conn` (250-261) are all `#[cfg(test)]`; non-test `lock_conn` is unchanged.
- No deadlock: `save` takes conn then `problem`; `load_problem` takes `problem` only; `open` neither. The hook runs with no lock held.
- Fail-closed unchanged: `read()` (149-176) and the `all_off` path untouched; `open` still logs at `error`; a failed save keeps the banner (tested).

blocking findings:
- none

design check: n/a

non-blocking findings:
- crates/tinwave-db/src/settings.rs:315-326 — If an assert or `unwrap` inside the `thread::scope` closure fails before `go_tx.send(())`, the test hangs instead of failing (thread A blocks in `go_rx.recv()`; `go_tx` only borrowed). No test timeout config in the repo. Fix: send `go` before the in-scope asserts, or move `go_tx` into a drop guard.
- crates/tinwave-db/tests/signal_settings.rs:452 — trigger SQL literal has a run of ~14 spaces (lost `\` continuation). Cosmetic.
- crates/tinwave-db/src/lib.rs:22 — the `text` doc example still reads correctly with the owned return. No action.

proposals:
- none new (P-0126 already parked).

notes for lead: `load_problem()` changed to `Option<SettingsProblem>`; only tinwave-db tests call it. Whoever wires the Signal Center should poll `load_problem()` after each save, not `load()`. The T-0021 non-blocking finding about the second writer connection is still open.
