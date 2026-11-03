---
id: P-0296
title: Derived kinds go with their inputs' tier on a tier wipe; seed RateSession in the wipe fixture
from: T-0064 review-1
date: 2026-11-03
size: S
outcome: merged → T-0070 (lead decision: a derived kind is cleared by the wipe of any tier or category it is derived from)
---
A T0 wipe keeps PerceivedLevel (T1) though it is computed from T0 volume events; the fixture has no RateSession row.
