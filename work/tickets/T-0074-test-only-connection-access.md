---
id: T-0074
title: Test-only connection access behind a feature; pin write_batch statement count
type: chore
epic: E-01
status: backlog
priority: P2
size: S
depends_on: [T-0073]
scope:
  - crates/tinwave-db/
owner:
branch:
review_round: 0
created_by: from P-0298, P-0299 (T-0073 build-1)
---

## Context
T-0073 added `#[doc(hidden)] pub fn connection_for_tests(&mut self) -> &mut Connection` on `SignalWiper` so integration
tests can count statements with rusqlite `trace_v2`. Any crate can call it, the desktop app included, which bypasses the
rule that the UI never touches SQLite.

## Scope
- Gate test-only access behind a `test-support` cargo feature of tinwave-db, enabled only as a dev-dependency feature (or
  move the counting into an in-crate `#[cfg(test)]` module); the default build exposes no raw connection.
- Count the statements of `SignalStore::write_batch` and pin them per batch size (fixed count independent of rows
  already stored; linear only where the batch itself requires).

## Acceptance criteria
- [ ] `cargo build -p tinwave-desktop` (default features) cannot reach a raw Connection through tinwave-db — shown.
- [ ] write_batch statement count pinned; a per-row mutation fails it.
- [ ] fmt, clippy, nextest, deny pass.

## Signals & storage
None.
