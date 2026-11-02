---
id: T-0015
title: Tidy after T-0013 — drop uuid v7, centralize getrandom, guard secrets from the Rng seam
type: chore
epic: E-00
status: done
priority: P2
size: S
depends_on: [T-0005]   # T-0013, T-0004 done; waits for T-0005 (same crate)
scope:
  - .gitattributes
  - Cargo.toml
  - Cargo.lock
  - crates/tinwave-core/Cargo.toml
  - crates/tinwave-core/src/rng.rs
  - crates/tinwave-core/src/id.rs
  - crates/tinwave-signals/Cargo.toml
  - crates/tinwave-signals/src/enums.rs
  - crates/tinwave-signals/src/frozen.rs
  - crates/tinwave-signals/src/coalesce.rs   # constant rename + merged-seek direction rule
  - crates/tinwave-signals/src/bus.rs        # one doc line on wait() during shutdown
  - crates/tinwave-signals/src/lib.rs        # re-export rename only
owner: tw-builder (round 1)
branch: t/T-0015-workspace-deps-tidy
review_round: 1
created_by: from P-0014, P-0015, P-0017, P-0019, P-0028
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
- `InputSource::NAMES` and its frozen list come from an exhaustive match (no wildcard), so a new variant fails to
  compile until it is named and frozen (P-0028).
- Merged seeks (T-0022 review non-blocking 1, lead decision): when sign(to − from) ≠ sign(summed jumps), suppress
  and count instead of emitting, so `SeekBack` always has `to < from` and `SeekForward` `to > from` (catalog §3).
  Tests: the reviewer's 1.5× and 3 × 0.8 s examples are suppressed and counted.
- bus.rs: document that `wait` sleeps through producer disconnect while a failed batch is retained (shutdown can be
  delayed by one flush interval only while the database fails). coalesce.rs:862 comment moved above its statement.
- Rename `SEEK_MIN_NET_DS` to `SEEK_MIN_TOTAL_DS` (it thresholds the summed jumps since T-0022; P-0038).
- Fix the doc label in id.rs: the algorithm is RFC 9562 §6.2 **Method 2** (monotonic random), not Method 1.

## Acceptance criteria
- [ ] `grep -rn "new_v7\|now_v7" crates/` is empty and the workspace builds — command output.
- [ ] `cargo tree -e features -i uuid` no longer shows `v7` — command output.
- [ ] A merged seek whose net movement disagrees with its jumps emits nothing and is counted — tests.
- [ ] `SeededRng` does not implement `CryptoRng`; `SystemRng` does — compile-time test.
- [ ] `git check-attr -a x.bat x.cmd` shows `eol: crlf`; the new binary patterns show `binary: set` — command output.
- [ ] fmt, clippy `-D warnings`, tests, `cargo deny check` pass.

## Signals & storage
None.

## Merged
2026-11-02, merge commit 85c7012 (branch head d2cbb3f). Review: review-1 (APPROVE). Gates green on main (174 tests + doc-test; emit p99 200-400 ns; contention run 0 reserved overflows on main).
