---
id: D-0006
title: Native-style findings from T-0011 — menus, confirmations, Qt floor, memory counter
status: decided
blocks: []
date: 2026-11-02
---

## Questions for the PO (from spike T-0011)
1. Windows menus: native + undocumented `SetPreferredAppMode` for dark mode, or Qt-drawn Fluent menus?
2. Confirmations: accept Qt's themed MessageDialog (no native message box in Qt on Windows/Linux)?
3. Qt floor: keep 6.8 LTS or raise to 6.10 (contrast theming, accessibility hints)?
4. Spec §12 memory targets: private working set or total working set?

## Context
work/reports/T-0011/spike-1.md, docs/adr/0007-native-style-findings.md, screenshots in spikes/T-0011-native-styles/.

## Lead recommendation
1 native + dark fix (fallback Popup.Window); 2 Qt themed dialog; 3 raise to 6.10; 4 private working set.

## PO decision
2026-11-02: all four as recommended. Applied: ADR 0007 (amends 0003), spec §2 and §12, design-system §2,
desktop-dev-setup.md, E-00 risks.
