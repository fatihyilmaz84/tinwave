---
id: P-0258
title: Compaction registers new rollup columns and play.flags sources in signal_wipe.rs
from: T-0061 build-1
date: 2026-11-03
size: S
outcome: parked: requirement for the compaction ticket (hot → warm → cold)
---
signal_wipe.rs `FLAG_SOURCES` / `ROLLUP_COLUMNS` are the single place the derivation rule lives.
