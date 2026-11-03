---
id: P-0281
title: Log the TRUNCATE busy cause correctly (writer vs reader)
from: T-0065 review-1
date: 2026-11-03
size: S
outcome: merged → T-0064
---
Right after a long wipe the writer usually holds the lock; PASSIVE already zeroed the main file, only the WAL keeps old
frames. Evidence: wipe.rs:301-303, privacy-and-signals.md:69-70.
