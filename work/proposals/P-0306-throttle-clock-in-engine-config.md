---
id: P-0306
title: Own the rate-throttle clock in EngineConfig
from: T-0063 build-1
date: 2026-11-03
size: S
outcome: accepted → T-0076
---
T-0063 throttles on monotonic time OR the injected event clock (hybrid). A dedicated throttle clock field would
remove the hybrid and let rt_alloc.rs:340 and controller_signals.rs advance a clock instead of sleeping 130 ms.
