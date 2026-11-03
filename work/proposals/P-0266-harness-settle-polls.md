---
id: P-0266
title: common::Harness::settle polls (no spin in exclusive mode)
from: T-0057 build-1
date: 2026-11-03
size: S
outcome: accepted → T-0066
---
`settle()` spins forever in exclusive mode when a load needs a reopen; tests must remember `settle_polling`.
