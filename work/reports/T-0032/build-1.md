HANDOFF T-0032 (round 1)
status: partial
branch: t/T-0032-settings-followups   commit: f188269
summary: Items 1 and 3 are done and tested. Item 2 uses the ticket's "documented" alternative, not a read-only type, because a type-level view is blocked by scope (details under deviations). The workspace is green.

acceptance criteria:
- [x] Item 1: `load_problem()` clears in-session after the first successful save.
  - `PersistentSettings` has a new `problem: Mutex<Option<SettingsProblem>>`, cleared inside `save()` after a successful upsert. The signature is now `load_problem(&self) -> Option<SettingsProblem>` (owned clone, not a reference). `load()` is unchanged and still describes startup.
  - Proof: `the_paused_banner_clears_after_the_first_successful_save_only` (a failed save keeps the banner; the first good save clears it; later saves keep it clear; `load()` still reports startup). `restored_settings_never_report_a_problem_before_or_after_a_save`. The missing-row and corrupt-row tests now also assert the banner clears in the same session.
- [~] Item 2: no read-only type, see deviations.
  - `PersistentSettings::settings()`, the module docs, and `SignalSettings::set_category` / `set_tier` now say: toggle only through `PersistentSettings`; direct calls are in-memory only and are lost at the next start.
  - Proof: docs only. The existing round-trip tests cover the supported route (row equals mask after toggling through `PersistentSettings`).
- [x] Item 3: deterministic test pinning that the upsert reads the mask under the connection lock.
  - Proof: unit test `settings::tests::the_mask_is_read_under_the_connection_lock` in `crates/tinwave-db/src/settings.rs`.
  - A `#[cfg(test)]` one-shot hook runs in `lock_conn()`, after the atomic update and before the lock is requested. Toggle A is parked there, toggle B runs to completion, then A resumes.
  - Mutation check: with `let mask = ...mask()` moved before `self.lock_conn()`, the test FAILS (`left: 894, right: 1918`; the row has lost B's tier). With the correct code it passes. A second test pins that the hook fires once.
- [x] fmt, clippy `-D warnings`, tests, `cargo deny check` pass. Intra-doc links also build: `RUSTDOCFLAGS="-D warnings" cargo doc -p tinwave-db -p tinwave-signals`.

checks: fmt pass, clippy pass (workspace, all targets), tests 397 passed / 0 failed (workspace, no audio device, no Credential Manager), deny pass
files changed: crates/tinwave-db/src/settings.rs, crates/tinwave-db/tests/signal_settings.rs, crates/tinwave-signals/src/settings.rs
signals added/changed: none
ui: none
docs updated: rustdoc only, in the three files above. `crates/tinwave-db/src/lib.rs` has a doc example using `if let Some(problem) = toggles.load_problem()`. It is out of scope and still compiles as pseudo-code with the owned return.
deviations from ticket:
- Item 2 is the "documented" alternative. A compile-time read-only view needs either a new `SettingsView` type exported from `crates/tinwave-signals/src/lib.rs` (outside scope) or taking the public mutators off `SignalSettings` (`SignalBus::settings()` returns `&Arc<SignalSettings>`; ~15 call sites in bus.rs, coalesce.rs, counters.rs, bus_acceptance.rs, tinwave-context tests and emit.rs call `set_category` / `set_tier` directly — all outside scope).
- `load_problem()` now returns `Option<SettingsProblem>` (owned) instead of `Option<&SettingsProblem>`. Interior mutability requires this.

proposals:
- P: Make the toggle mutators unreachable from producers | why: only the docs stop a producer from calling `set_*` on the shared `Arc<SignalSettings>`, which changes the mask without persisting it | size: M | evidence: crates/tinwave-signals/src/bus.rs:299; crates/tinwave-context/src/audio/engine_tests.rs:755; crates/tinwave-signals/tests/bus_acceptance.rs:173.

questions for lead:
- none

---
Lead (2026-11-02): item 2's documented alternative is explicitly allowed by the ticket ⇒ treated as done. Proposal → P-0126 (parked).
