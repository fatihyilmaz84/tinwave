---
id: P-0253
title: Skip the flush-wait backoff for rate restarts
from: T-0027 review-1
date: 2026-11-03
size: S
outcome: accepted → T-0063
---
decoder.rs:233-242 backs off 1->2->4->8->16 ms while awaiting the ack, stretching the dip by up to ~15 ms.
