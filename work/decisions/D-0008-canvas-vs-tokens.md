---
id: D-0008
title: Should the design canvas follow the token table exactly (surfaces, macOS/Linux icon size)?
status: open
blocks: []
date: 2026-11-03
---

## Questions for the PO (from T-0046)
1. Several artboards use surface colours close to, but not equal to, the tokens (e.g. Library-Windows sidebar #EDEFF3 vs
   token `sidebar` #ECECEF; SignalCenter-Windows window #1C1C1F vs #161618; Album-macOS sidebar #232326 vs #1D1D20).
   The app always uses the tokens. Update the artboards to the tokens, or are those differences intentional?
2. The macOS and Linux artboards draw sidebar icons at 16 px in 28 px rows; the token `iconSize` is 20 (and Windows rows
   are 34 px). Keep a compact sidebar on macOS/Linux (a new `iconSizeDense`-style row), or use 20 px everywhere?

## Context
work/reports/T-0046/build-1.md (file:line list); docs/design/design-system.md §3.1, §3.4.

## Lead recommendation
1. Update the artboards to the tokens (the tokens are the source of truth; near-miss values only cause doubt later).
2. Keep 20 px everywhere until the macOS/Linux spikes (T-0038/T-0039) show a native reason for a compact sidebar.

## PO decision
_pending_
