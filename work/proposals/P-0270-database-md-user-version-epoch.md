---
id: P-0270
title: database.md: PRAGMA user_version is the signals.db wipe epoch
from: T-0065 build-1
date: 2026-11-03
size: S
outcome: merged → T-0065 (lead applied on merge 4950c97: database.md "Reserved pragmas")
---
A future migration author must not reuse it. Evidence: signal_wipe.rs `wipe_epoch`.
