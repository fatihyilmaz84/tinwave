---
id: P-0017
title: Guard secrets from the deterministic Rng seam (CryptoRng marker)
from: T-0013 reviewer
date: 2026-11-01
size: S
outcome: merged → T-0015
---
## Why
`SeededRng` (SplitMix64) satisfies `Rng`; a future `fn make_secret(&mut impl Rng)` would compile with it.
