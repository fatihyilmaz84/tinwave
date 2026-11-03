---
id: P-0295
title: Pin the wipe's cost with a statement count
from: T-0064 review-1
date: 2026-11-03
size: S
outcome: accepted → T-0073
---
The contention tests no longer bound wipe time; a per-row regression in the wipe SQL would go unnoticed.
