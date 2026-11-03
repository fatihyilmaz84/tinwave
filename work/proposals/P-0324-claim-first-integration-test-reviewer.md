---
id: P-0324
title: Integration test for the claim-first interleaving (reviewer); also cover the engine's lost-claim branch
from: T-0068 review-1
date: 2026-11-03
size: S
outcome: merged → T-0081 (duplicate of P-0323)
---
Same as P-0323. The reviewer adds: mutation `if !take_reopen(..) && id == u32::MAX` in engine.rs ~915 survives every test.
