REVIEW T-0001 (round 2)
verdict: APPROVE
checks (run by reviewer, worktree D:\dev\Tinwave-worktrees\T-0001-workspace-and-core-ids @ bff7afb, clean status, rustc/cargo 1.92.0): fmt pass (exit 0), clippy pass (`--workspace --all-targets -- -D warnings`, exit 0), tests pass (`cargo test --workspace`: 21 passed / 0 failed), deny pass ("advisories ok, bans ok, licenses ok, sources ok", exit 0, no more `license-not-encountered` warnings)

The builder's claim is accurate. `git diff dab50d6..bff7afb` changes exactly two lines: `Cargo.toml:6` now reads `exclude = ["spikes"]`, and `deny.toml:15` adds `unused-allowed-license = "allow"`. `git diff --stat main...HEAD` still lists the same 13 files, so no probe or spikes/ files were committed.

acceptance criteria:
- [x] `cargo build --workspace` and `cargo test --workspace` pass — verified by: my own runs, exit 0, 21/0.
- [x] fmt and clippy `-D warnings` pass — verified by: my own runs, exit 0.
- [x] `cargo deny check` passes with the license policy — verified by: my run, exit 0. Under licenses v2, everything outside the allow list (MIT, Apache-2.0, BSD-2/3, ISC, Zlib, Unicode) is still denied, so GPL, AGPL and LGPL stay denied. Advisories are still on. The `unused-allowed-license` change only silences unused-allowance warnings and does not loosen the policy.
- [x] Ids are time-ordered UUIDv7 and SignalEventId orders by (device, seq) — verified by: id.rs tests `generated_ids_are_uuid_v7`, `ids_generated_over_time_sort_in_creation_order`, `ids_in_a_tight_loop_are_unique_and_monotonic`, `new_at_embeds_and_orders_by_the_given_time`, `signal_event_id_orders_by_device_then_seq` (unchanged since round 1, all pass).
- [x] Ids implement Copy/Eq/Ord/Hash/Debug/Display and serde round-trips — verified by: `all_ids_implement_required_traits`, `uuid_ids_serde_round_trip_as_plain_strings`, `signal_event_id_serde_round_trip_and_display`, `display_and_parse_round_trip`.
- [x] No unwrap/expect outside tests — verified by: round-1 grep. The code is unchanged, and the workspace `unwrap_used`/`expect_used` lints plus clippy `-D warnings` still enforce it.
- [x] Ticket scope "spike prototypes never join the workspace" — verified by: I copied bff7afb (`git archive`) to a scratch dir and added a plain `spikes/probe` with no `[workspace]` table. `cargo build` in it now succeeds (exit 0, round 1 was exit 101). Its `cargo metadata` workspace_root is the probe itself. Root `cargo metadata --no-deps` lists only tinwave-core as a workspace member. Root `cargo build --workspace` passes. I deleted the scratch copy afterwards and confirmed it is gone.

blocking findings:
- none

design check: n/a

non-blocking findings:
- none new. The round-1 items (Clock/Rng/Default on ids, Cargo.lock scope, exclude wording) are tracked as you said: T-0013 and the ticket edits.

proposals:
- none new

notes for lead: Safe to merge. Per the DoD, re-run fmt, clippy, tests and deny on main after the merge. Once this lands, the parallel spikes under `spikes/` will be excluded correctly whether or not they have their own `[workspace]` table. I did not touch D:\dev\Tinwave or the other worktrees.
