---
id: P-0002
title: Injectable Clock trait in tinwave-core, used for id generation
from: T-0001 builder
date: 2026-11-01
size: S
outcome: accepted → T-0013
---

## Why
docs/rules/testing.md:26 forbids `SystemTime::now()` in domain code; `RecordingId::new()` reads the clock.
T-0005 also needs an injected `Clock`; it belongs in `tinwave-core` so every crate shares one.

## Evidence
crates/tinwave-core/src/id.rs — `Uuid::now_v7()` in `new`.

## Suggested approach
`Clock` trait (`now() -> UtcMillis`), `SystemClock`, a manual test clock; id constructors take a clock.
