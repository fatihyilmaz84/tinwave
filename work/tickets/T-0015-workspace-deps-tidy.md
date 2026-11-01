---
id: T-0015
title: Tidy workspace dependencies after T-0013 (drop uuid v7, centralize getrandom)
type: chore
epic: E-00
status: backlog
priority: P2
size: S
depends_on: [T-0013, T-0004]
scope:
  - Cargo.toml
  - Cargo.lock
  - crates/tinwave-core/Cargo.toml
  - crates/tinwave-signals/Cargo.toml
owner:
branch:
review_round: 0
created_by: from P-0014, P-0015
---

## Context
T-0013 builds UUIDv7 ids by hand from an injected clock and rng, so uuid's own `v7` feature (and its getrandom path)
is dead weight. `getrandom` is a direct dependency of tinwave-core only. docs/rules/rust.md (dependencies).

## Scope
- Remove `v7` from the workspace `uuid` features (keep what is still used, e.g. `serde`).
- Move `getrandom` into `[workspace.dependencies]`; crates use `getrandom.workspace = true`.

## Acceptance criteria
- [ ] `grep -rn "new_v7\|now_v7" crates/` is empty and the workspace builds — command output.
- [ ] `cargo tree -e features -i uuid` no longer shows `v7` — command output.
- [ ] fmt, clippy `-D warnings`, tests, `cargo deny check` pass.

## Signals & storage
None.
