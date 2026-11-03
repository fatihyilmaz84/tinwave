---
id: P-0270
title: database.md: PRAGMA user_version is the signals.db wipe epoch
from: T-0065 build-1
date: 2026-11-03
size: S
outcome: accepted → lead applies on T-0065 merge
---
A future migration author must not reuse it. Evidence: signal_wipe.rs `wipe_epoch`.
