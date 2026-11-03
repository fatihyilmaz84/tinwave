---
id: P-0256
title: Per-device high-water seq that survives wipe everything
from: T-0061 build-1
date: 2026-11-03
size: M
outcome: parked: requirement for E-05 sync (no sync peer exists yet); recorded in T-0061 lead notes
---
After wipe everything + restart `SignalStore::first_seq` returns 0, so a sync peer holding (device, 0..n) would dedupe
new events away. Evidence: store.rs `first_seq`.
