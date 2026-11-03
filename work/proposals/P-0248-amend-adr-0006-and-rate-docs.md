---
id: P-0248
title: Amend ADR 0006 and the rate docs (catalog §2, architecture.md, spec §7)
from: T-0027 build-1
date: 2026-11-03
size: S
outcome: merged → T-0027 (lead applied on merge 9a3d07b: ADR 0006 implementation notes, catalog §2, architecture.md; spec §7 needs no change)
---
Record the five deviations (no segment queue, Mark.rate positions, pre-roll, multi-callback fade-out, 125 ms throttle)
and replace "step equals steady sine" with "+0–2 % stretcher ripple". Evidence: docs/adr/0006-time-stretch.md decisions 4–6.
