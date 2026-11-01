HANDOFF T-0001 (round 2)
status: done
branch: t/T-0001-workspace-and-core-ids   commit: bff7afb
summary: I fixed the blocking finding: `Cargo.toml` now has `exclude = ["spikes"]`, a path prefix, instead of the glob that did nothing. Your probe check now passes. I also added `unused-allowed-license = "allow"` to `deny.toml`, so `cargo deny check` no longer prints license-not-encountered warnings. Nothing else changed.

acceptance criteria:
- [x] `cargo build --workspace` and `cargo test --workspace` pass. Proof: 21 passed / 0 failed.
- [x] `cargo fmt --all --check` and `cargo clippy --workspace --all-targets -- -D warnings` pass. Proof: fmt printed no diff, clippy finished clean.
- [x] `cargo deny check` passes. Proof: "advisories ok, bans ok, licenses ok, sources ok", now without warnings.
- [x] UUIDv7 ordering, `SignalEventId` ordering, trait and serde round-trip tests are unchanged and still pass (round 1 test names).
- [x] No `unwrap`/`expect` outside tests. Proof: clippy lint `unwrap_used`/`expect_used`, unchanged.
- [x] Round-2 fix verified as you specified:
  - I created a plain `spikes/probe` package with no `[workspace]` table.
  - `cargo build` inside `spikes/probe` succeeded standalone.
  - `cargo metadata --no-deps` at the root listed only `tinwave-core`.
  - I deleted `spikes/probe` before committing; `git status` showed only `Cargo.toml` and `deny.toml` modified.

checks: fmt pass, clippy pass, tests 21 passed / 0 failed, deny pass
files changed: Cargo.toml, deny.toml
signals added/changed: none
ui: none
docs updated: none
deviations from ticket: unchanged from round 1 (`members = ["crates/*"]` only, because Cargo errors on a missing `apps/` glob; `Cargo.lock` committed). `exclude = ["spikes"]` replaces the ticket's `["spikes/*"]` because Cargo does not expand globs in `exclude`.

proposals: unchanged from round 1 (`.gitattributes`, `Clock` trait covered by T-0013, CI workflow).

questions for lead: none
