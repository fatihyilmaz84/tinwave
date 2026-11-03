---
id: P-0247
title: Context hub writes snapshot rows only through the writer transaction and handles hash/key divergence after a wipe
from: T-0060 review-1
date: 2026-11-03
size: S
outcome: parked: requirement for the context-hub ticket (E-03), with P-0236 and P-0245
---
A snapshot built from pre-wipe values and inserted after a wipe would reintroduce wiped fields; `context_snapshot.hash`
(UNIQUE, from the quantized fields) no longer matches a wiped `key`, so dedupe by hash could reuse a wiped row.
Evidence: crates/tinwave-db/src/wipe.rs:39-43, 153-158.
