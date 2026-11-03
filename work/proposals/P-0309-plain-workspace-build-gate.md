---
id: P-0309
title: Run a plain (non-test) workspace build in the gates
from: T-0074 review-1
date: 2026-11-03
size: S
outcome: accepted → lead gate: `cargo build --workspace` (already in CLAUDE.md commands) runs on every merge from T-0074 on
---
Under resolver 2, `clippy --all-targets` and `nextest --workspace` unify tinwave-db's `test-support` dev feature into
every consumer, so only a plain build catches a product call to `connection_for_tests`.
