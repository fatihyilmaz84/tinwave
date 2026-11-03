---
id: P-0271
title: Wiper nudges the writer's checkpoint clock after a wipe
from: T-0065 build-1
date: 2026-11-03
size: S
outcome: parked: harmless (the writer's next idle checkpoint of an empty WAL is a no-op)
---
Evidence: store.rs `checkpoint_if_due`.
