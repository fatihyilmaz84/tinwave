---
id: P-0285
title: Re-export exclusive_refusal from lib.rs
from: T-0066 build-1
date: 2026-11-03
size: S
outcome: accepted → lead applies on T-0066 merge
---
Out-of-crate AudioOutput implementors can't return the typed refusal. Evidence: lib.rs:61, output/mod.rs.
