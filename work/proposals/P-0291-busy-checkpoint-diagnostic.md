---
id: P-0291
title: Expose the busy-checkpoint outcome in Diagnostics
from: T-0064 build-1
date: 2026-11-03
size: S
outcome: parked: requirement for the Signal Center wipe action, with P-0244
---
`Checkpoint::Busy { main_file_current }` is only logged; the listener can't see that old content may sit in the WAL.
