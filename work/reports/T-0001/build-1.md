HANDOFF T-0001 (round 1)
status: done
branch: t/T-0001-workspace-and-core-ids   commit: dab50d6
summary: I added the virtual Cargo workspace and the quality-gate configs (toolchain pin, rustfmt, clippy, deny, gitignore). I also added the `tinwave-core` crate with UUIDv7-backed ids, `SignalEventId`, `Tier`, `UtcMillis` and `TzOffsetMin`. Build, tests, fmt, clippy `-D warnings` and `cargo deny check` all pass. `members` is `["crates/*"]` only, not `crates/*` plus `apps/*`, because Cargo rejects the missing `apps/` glob.

acceptance criteria:
- [x] `cargo build --workspace` and `cargo test --workspace` pass. Proof: both ran clean, 21 passed / 0 failed. The first run also installed toolchain 1.92.0 from `rust-toolchain.toml`.
- [x] `cargo fmt --all --check` and `cargo clippy --workspace --all-targets -- -D warnings` pass. Proof: fmt printed no diff, clippy finished clean.
- [x] `cargo deny check` passes with the license policy. Proof: "advisories ok, bans ok, licenses ok, sources ok". There are only warnings for allowed licenses no crate uses yet (BSD, ISC, Zlib, Unicode-DFS-2016).
- [x] UUIDv7 generation is time-ordered, and `SignalEventId` orders by `(device, seq)`. Proof: unit tests `generated_ids_are_uuid_v7`, `ids_generated_over_time_sort_in_creation_order`, `ids_in_a_tight_loop_are_unique_and_monotonic`, `new_at_embeds_and_orders_by_the_given_time`, `signal_event_id_orders_by_device_then_seq`.
- [x] Ids implement `Copy, Eq, Ord, Hash, Debug, Display` and round-trip through serde. Proof: `all_ids_implement_required_traits` is a compile-time bound check. Round-trip tests are `uuid_ids_serde_round_trip_as_plain_strings`, `signal_event_id_serde_round_trip_and_display` and `display_and_parse_round_trip`.
- [x] No `unwrap`/`expect` outside tests. Proof: the workspace lints set `clippy::unwrap_used` and `clippy::expect_used` to warn, and clippy `-D warnings` passes. `clippy.toml` allows both in tests.

checks: fmt pass, clippy pass, tests 21 passed / 0 failed, deny pass
files changed: Cargo.toml, Cargo.lock, rust-toolchain.toml, rustfmt.toml, clippy.toml, deny.toml, .gitignore, crates/tinwave-core/Cargo.toml, crates/tinwave-core/src/{lib,error,id,time,tier}.rs
signals added/changed: none
ui: none
docs updated: none

deviations from ticket:
- `members = ["crates/*"]`, not `["crates/*", "apps/*"]`. With no `apps/` directory, Cargo errors ("failed to read apps\*\Cargo.toml", os error 3). A comment in `Cargo.toml` says to add `apps/*` when the folder exists. This needs your attention: whoever creates `apps/tinwave-desktop` must add `apps/*` to `members`.
- `Cargo.lock` is committed. It is not in the ticket's scope list, but it is generated and needed for reproducible builds and `cargo deny`.
- Beyond the ticket, I added `[workspace.package]` and `[workspace.lints]` (forbid unsafe, warn on `unwrap_used` and `expect_used`). I also added `CoreError` (thiserror) and a few small helpers: `new_at(UtcMillis)`, `created_at()`, `from_bytes`/`as_bytes`, `Tier::{ALL, number, is_opt_in}` and `TzOffsetMin::{MIN, MAX, UTC}`.
- Design choices you may want to know about:
  - Ids reject non-v7 UUIDs on parse and deserialize.
  - `TzOffsetMin` validates -720..=840 and rejects out-of-range values when deserializing.
  - `RecordingId::new()` uses the system clock. This is the only place `tinwave-core` reads the clock, and `new_at(UtcMillis)` is the deterministic path for injected clocks.
- `rustfmt.toml` sets only `edition`. I did not set `newline_style = "Unix"` because `core.autocrlf=true` would turn CRLF checkouts into `fmt --check` failures. rustfmt's default Auto newline detection avoids that, and fmt passes.
- `spikes/*` exclusion: I created a throwaway `spikes/x` package and `cargo metadata` listed only `tinwave-core` as a workspace member. I then deleted it.
- `tinwave-core` dev-depends on `serde_json` (tests only). Runtime dependencies are just `uuid`, `serde` and `thiserror`.

proposals:
- P: Add `.gitattributes` (`* text=auto eol=lf`, plus `*.rs text eol=lf`) | why: pins line endings so fmt, clippy and diffs are stable across Windows autocrlf, macOS and Linux | size: S | evidence: git warns "LF will be replaced by CRLF" on every committed file in this branch
- P: Add an injectable `Clock` trait (in `tinwave-core`) and use it for id generation | why: testing.md forbids `SystemTime::now()` in domain code, and `new()` currently reads the clock | size: S | evidence: crates/tinwave-core/src/id.rs (`Uuid::now_v7()` in `new`)
- P: Add a CI workflow running fmt, clippy, nextest and `cargo deny check` | why: desktop-dev-setup.md "Before every PR" lists these gates but nothing enforces them yet | size: S | evidence: docs/guides/desktop-dev-setup.md:46-53

questions for lead:
- none
