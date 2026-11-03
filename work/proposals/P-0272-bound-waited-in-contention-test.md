---
id: P-0272
title: Contention test asserts a bound on the writer's wait
from: T-0065 build-1
date: 2026-11-03
size: S
outcome: merged → T-0064
---
Only success within 5 s is proven; waited 0.65-0.77 s quiet, up to 3.3 s with 4 parallel copies — a growing stall is
invisible until it flakes. Evidence: tests/signal_wipe.rs `a_write_batch_during_a_wipe_of_the_reference_tables_is_stored`.
