---
id: P-0259
title: Invalidate writer caches only after a wipe, not every foreign commit
from: T-0061 review-1
date: 2026-11-03
size: S
outcome: accepted → T-0065
---
data_version fires on every other-connection commit: 4 347 clears and ~+1.7 s writer CPU (~+30 %) in the 90-day harness.
