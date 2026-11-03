---
id: P-0265
title: Engine wakes its host when a reopen is pending
from: T-0057 build-1
date: 2026-11-03
size: S
outcome: parked: the controller's 10 ms fast poll covers the only host today; revisit if another host owns the loop
---
Evidence: engine.rs `Engine::reopen_pending`.
