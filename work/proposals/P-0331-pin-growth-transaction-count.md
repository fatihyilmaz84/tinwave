---
id: P-0331
title: Pin the growth run's transaction count (a derived value never adds a commit)
from: T-0070 review-1
date: 2026-11-04
size: S
outcome: accepted → T-0084
---
The harness prints 11 471 transactions but does not assert it; only writer unit tests pin it.
