---
id: P-0241
title: Pair a start stored after its outcome (synced/imported streams)
from: T-0029 review-1
date: 2026-11-03
size: S
outcome: parked: E-05 sync (same-device order is guaranteed by the reserved FIFO today)
---
Today only a newly stored outcome writes a row; a synced stream may deliver the start later.
