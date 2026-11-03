---
id: P-0234
title: Document end_reason thresholds and PlayFlags bits in storage-budget.md / database.md
from: T-0029 build-1
date: 2026-11-03
size: S
outcome: accepted → lead applies on T-0029 merge
---
Thresholds (SkippedEarly < 30 s listened, SkippedLate > 80 %) and the flag bit order live only in Rust docs
(crates/tinwave-db/src/play.rs `EndReason`, `PlayFlags`).
