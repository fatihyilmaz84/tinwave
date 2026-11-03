---
id: P-0320
title: Pin the ack reset of the flush poll state
from: T-0063 review-2
date: 2026-11-03
size: S
outcome: accepted → T-0080
---
Removing `flush_wait = AckPoll::new()` (decoder.rs ~375) passes every test, yet it keeps rate restarts on fast polling.
