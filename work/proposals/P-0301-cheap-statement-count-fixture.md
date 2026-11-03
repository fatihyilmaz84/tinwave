---
id: P-0301
title: Make the signal-wipe statement-count fixture cheap
from: T-0073 review-1
date: 2026-11-03
size: S
outcome: merged → T-0074
---
`a_wipe_runs_a_fixed_number_of_statements_whatever_the_row_count` is now the slowest test (84.6 s under load, 11-13 s
alone); `populate()` runs ~100 autocommit inserts 32 times. One transaction roughly halves it.
