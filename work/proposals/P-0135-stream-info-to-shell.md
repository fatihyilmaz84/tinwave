---
id: P-0135
title: Expose the live StreamInfo / stream mode to the shell
from: T-0034 build-1
date: 2026-11-02
size: S
outcome: parked — first controller shell-wiring / Diagnostics ticket
---
The mapper sees OutputOpened/OutputReopened, but the shell only observes them through signals; Diagnostics needs the live mode and fallback reason.
