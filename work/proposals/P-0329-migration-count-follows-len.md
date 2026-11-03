---
id: P-0329
title: signal_store.rs migration-count asserts follow SIGNALS_MIGRATIONS.len()
from: T-0070 build-1
date: 2026-11-03
size: S
outcome: merged → T-0070 (lead applied on merge, 382e063)
---
Every migration breaks the hard-coded count and the play column list (tests/signal_store.rs:81-90).
