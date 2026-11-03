---
id: P-0325
title: Hold the engine's claim back while a replace_after is unanswered (reviewer)
from: T-0068 review-1
date: 2026-11-03
size: S
outcome: merged → T-0081 (duplicate of P-0322)
---
Same as P-0322. The reviewer adds: mutation `if !take_reopen(..) && id == u32::MAX` in engine.rs ~915 survives every test.
