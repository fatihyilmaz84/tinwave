---
id: P-0004
title: Injectable Clock + Rng for id generation; drop Default on ids
from: T-0001 reviewer
date: 2026-11-01
size: S
outcome: merged → T-0013
---

## Why
testing.md:26 determinism. `new_at` still draws 74 random bits from getrandom, so replay/golden tests
(testing.md:11-13) cannot pin ids. `Default` on ids makes any `#[derive(Default)]` struct silently read clock + RNG.

## Evidence
crates/tinwave-core/src/id.rs:29-31 (`new`), id.rs:95-99 (`Default`) on branch t/T-0001 @ dab50d6.

## Suggested approach
`new_at_with(at, &mut impl Rng)` or `from_parts(at, [u8; 10])`; remove `Default` impls.
