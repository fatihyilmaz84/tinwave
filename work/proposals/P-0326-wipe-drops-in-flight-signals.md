---
id: P-0326
title: Link SignalWiper to the bus so a wipe also drops in-flight values and events
from: T-0070 build-1
date: 2026-11-03
size: S
outcome: accepted → T-0082
---
A value or event queued or staged (up to one flush interval) when a wipe commits is still stored afterwards.
A wipe generation on `SignalSettings` that the wiper bumps and the drain/stage compare (ADR 0010 Consequences).
