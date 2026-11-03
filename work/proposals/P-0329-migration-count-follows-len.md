---
id: P-0329
title: signal_store.rs migration-count asserts follow SIGNALS_MIGRATIONS.len()
from: T-0070 build-1
date: 2026-11-03
size: S
outcome: accepted → lead applies on T-0070 merge (with the builder's signal_store.rs patch)
---
Every migration breaks the hard-coded count and the play column list (tests/signal_store.rs:81-90).
