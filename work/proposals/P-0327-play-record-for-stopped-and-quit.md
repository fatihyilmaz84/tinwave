---
id: P-0327
title: Write a play record for stopped and quit plays
from: T-0070 build-1
date: 2026-11-03
size: S
outcome: accepted → T-0083
---
Their RateSession is counted and dropped (`WriterStats::derived_unmatched`); the same gap blocks
`EndReason::Stopped`/`Quit` (play.rs module docs).
