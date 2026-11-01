---
id: T-0015
title: Tidy after T-0013 — drop uuid v7, centralize getrandom, guard secrets from the Rng seam
type: chore
epic: E-00
status: backlog
priority: P2
size: S
depends_on: [T-0013, T-0004]
scope:
  - .gitattributes
  - Cargo.toml
  - Cargo.lock
  - crates/tinwave-core/Cargo.toml
  - crates/tinwave-core/src/rng.rs
  - crates/tinwave-core/src/id.rs
  - crates/tinwave-signals/Cargo.toml
owner:
branch:
review_round: 0
created_by: from P-0014, P-0015, P-0017, P-0019
---

## Context
T-0013 builds UUIDv7 ids by hand from an injected clock and rng, so uuid's own `v7` feature (and its getrandom path)
is dead weight. `getrandom` is a direct dependency of tinwave-core only. docs/rules/rust.md (dependencies).

## Scope
- Remove `v7` from the workspace `uuid` features (keep what is still used, e.g. `serde`).
- Move `getrandom` into `[workspace.dependencies]`; crates use `getrandom.workspace = true`.
- Secrets guard (P-0017): a `CryptoRng` marker trait implemented by `SystemRng` only (not `SeededRng`), documented on
  `trait Rng` as required for any secret (e.g. `device_secret`); a compile-time test that `SeededRng: !CryptoRng`.
- `.gitattributes` (P-0019): `*.bat text eol=crlf`, `*.cmd text eol=crlf`; `binary` for jpeg, gif, webp, ttf, otf, aac,
  opus, sqlite, db-wal, db-shm.
- Fix the doc label in id.rs: the algorithm is RFC 9562 §6.2 **Method 2** (monotonic random), not Method 1.

## Acceptance criteria
- [ ] `grep -rn "new_v7\|now_v7" crates/` is empty and the workspace builds — command output.
- [ ] `cargo tree -e features -i uuid` no longer shows `v7` — command output.
- [ ] `SeededRng` does not implement `CryptoRng`; `SystemRng` does — compile-time test.
- [ ] `git check-attr -a x.bat x.cmd` shows `eol: crlf`; the new binary patterns show `binary: set` — command output.
- [ ] fmt, clippy `-D warnings`, tests, `cargo deny check` pass.

## Signals & storage
None.
