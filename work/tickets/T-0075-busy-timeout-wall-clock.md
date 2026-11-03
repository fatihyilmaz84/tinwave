---
id: T-0075
title: Contention tests that do not equate the SQLite busy timeout with wall time
type: chore
epic: E-01
status: backlog
priority: P3
size: S
depends_on: [T-0074]
scope:
  - crates/tinwave-db/tests/
owner:
branch:
review_round: 0
created_by: from P-0304 (T-0074 build-1)
---

## Context
SQLite's busy timeout is the sum of its back-off sleeps. On a loaded Windows machine each sleep runs long, so a 5 s
busy timeout took 6.1-6.4 s of wall time (T-0074). T-0074 fixed one test that released its lock after a fixed
`BUSY_TIMEOUT + 1 s` by waiting for the first attempt's busy error instead. Other contention tests (T-0073's, and
`common::AFTER_WIPE_SLACK`) may still assume the two are equal.

## Scope
- Grep `crates/tinwave-db/tests/` for fixed waits derived from the busy timeout; replace each with an event the test
  can observe (an attempt's busy error, a counter, a channel), or justify the margin in a comment.
- No production code change unless a test shows a real bug (then stop and report).

## Acceptance criteria
- [ ] Every fixed wait tied to the busy timeout is replaced or justified — listed in the handoff.
- [ ] The touched tests pass 10 of 10 runs under load (e.g. 4 nextest processes at once).
- [ ] fmt, clippy, nextest, deny pass.

## Signals & storage
None.
