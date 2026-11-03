---
id: P-0304
title: Busy-timeout assumptions that equate SQLite's busy timeout with wall time
from: T-0074 build-1
date: 2026-11-03
size: S
outcome: accepted → T-0075
---
SQLite's 5 s busy timeout is a sum of back-off sleeps; on loaded Windows it took 6.1-6.4 s. Tests using
`BUSY_TIMEOUT + n s` or `common::AFTER_WIPE_SLACK` can flake the same way the retry test did.
