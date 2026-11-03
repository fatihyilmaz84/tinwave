---
id: P-0249
title: Derive RateSession in the controller
from: T-0027 build-1
date: 2026-11-03
size: S/M
outcome: accepted → T-0062
---
Per play, snapshot `rate_stats()` at PlayStarted and at the leave, emit `time_weighted_rate_since`.
Evidence: controller/mod.rs `rate_stats()`, engine.rs `RateStats::time_weighted_rate_since`.
