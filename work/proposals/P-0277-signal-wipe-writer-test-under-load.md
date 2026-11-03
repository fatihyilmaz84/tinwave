---
id: P-0277
title: signal_wipe writer-thread test robust under parallel load
from: T-0062 build-1
date: 2026-11-03
size: S
outcome: merged → T-0064
---
`the_writer_thread_keeps_working_while_everything_is_wiped` failed once (16.9 s) in a fail-fast run alongside another
agent's tinwave-db nextest; passed on rerun.
