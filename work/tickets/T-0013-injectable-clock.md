---
id: T-0013
title: Add an injectable Clock and Rng seam to tinwave-core and use them for id generation
type: chore
epic: E-00
status: ready
priority: P0
size: S
depends_on: [T-0001]
scope:
  - crates/tinwave-core/
owner:
branch:
review_round: 0
created_by: from P-0002, P-0004
---

## Context
docs/rules/testing.md:26: inject `Clock` everywhere; no `SystemTime::now()` in domain code. T-0001 left
`RecordingId::new()` (and the other UUIDv7 ids) reading the system clock via `Uuid::now_v7()`. T-0005 needs an
injected `Clock` too; one shared trait in `tinwave-core` avoids each crate inventing its own.
docs/rules/rust.md, docs/rules/architecture.md.

## Scope
- `Clock` trait in `tinwave-core`: `fn now(&self) -> UtcMillis`, `Send + Sync`.
- `SystemClock` (the only place in `tinwave-core` that reads the system time) and `ManualClock` for tests
  (settable/advanceable, thread-safe), available to other crates' tests (feature `test-clock` or always-on —
  builder's call, record it).
- UUIDv7 ids are generated from a `&dyn Clock` (or generic `C: Clock`), keeping monotonic ordering within the
  same millisecond. Remove the clock-reading `new()` or make it delegate to `SystemClock` — no other system-time reads.
- Randomness seam (P-0004): ids can be built from explicit random bits (e.g. `from_parts(at, [u8; 10])` or
  `new_at_with(at, &mut impl RngCore)`) so replay/golden tests (testing.md:11-13) can pin ids; the getrandom path
  stays only behind the system/default generator.
- Remove the `Default` impls on id types (a `#[derive(Default)]` on a struct holding an id would silently read the
  clock and RNG).

## Out of scope
Monotonic/elapsed clocks for playback position (audio backend owns that). Any other crate.

## Acceptance criteria
- [ ] `grep -rn "now_v7\|SystemTime::now\|Instant::now" crates/tinwave-core/src` only matches `SystemClock` — verified by command output.
- [ ] Ids generated from a `ManualClock` embed exactly its time; advancing the clock orders ids — unit tests.
- [ ] Ids generated at the same `ManualClock` instant are unique and strictly increasing — unit test.
- [ ] Same clock time + same random bits ⇒ identical id, byte for byte — unit test.
- [ ] No id type implements `Default` — compile-time check or grep.
- [ ] fmt, clippy `-D warnings`, tests, `cargo deny check` pass.

## Signals & storage
None. Enables deterministic signal timestamps in T-0005 and the growth test (T-0007).

## Notes for the builder
Reuse `UtcMillis` and `new_at` from T-0001. Keep `tinwave-core` dependency-light.
