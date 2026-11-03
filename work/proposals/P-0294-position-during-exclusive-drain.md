---
id: P-0294
title: Heard position freezes during an exclusive drain, then resumes after the reopen
from: T-0066 review-1
date: 2026-11-03
size: S
outcome: merged → T-0068
---
On ManualOutput item 1's frame stopped at 16588 for the whole drain while the device still played its buffered tail,
then advanced again once the new stream ran: idle frames during the drain are ignored by the heard-position model.
