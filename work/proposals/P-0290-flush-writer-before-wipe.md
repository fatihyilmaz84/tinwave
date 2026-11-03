---
id: P-0290
title: Signal Center flushes the writer before "wipe now"
from: T-0064 build-1
date: 2026-11-03
size: S
outcome: parked: requirement for the Signal Center wipe action, with P-0244
---
Events buffered in the bus at wipe time are stored after the wipe (by design), so the last ~2 s survive it.
