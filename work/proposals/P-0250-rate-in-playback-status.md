---
id: P-0250
title: Expose the applied rate in PlaybackStatus
from: T-0027 build-1
date: 2026-11-03
size: S
outcome: merged → T-0062
---
The UI should show the engine's applied rate, not its own guess. Evidence: controller/types.rs `PlaybackStatus`.
