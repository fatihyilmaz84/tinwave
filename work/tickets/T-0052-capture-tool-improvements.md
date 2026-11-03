---
id: T-0052
title: capture.ps1 improvements — desktop-crop mode, click and key-down steps, cursor restore
type: chore
epic: E-01
status: ready
priority: P2
size: S
depends_on: [T-0036]
scope:
  - apps/tinwave-desktop/tools/capture.ps1
owner:
branch:
review_round: 0
created_by: from P-0175, P-0182, P-0186, P-0189
---

## Context
UI tickets keep needing scratch copies of tools/capture.ps1: T-0050 (click and held-key probes), T-0036 (PrintWindow
cannot capture Mica or native menus, so a desktop crop was used). The cursor is never restored, and when the window
covers every screen corner it is silently not parked.

## Scope
- `-Mode window|desktop`: desktop mode grabs the screen area of the DWM frame bounds (for Mica and native menus).
- Step tokens for mouse clicks at window-relative logical points and for key-down / key-up, plus a short wait token.
- Restore the cursor position at the end; warn when it cannot be parked outside the window.
- A readiness check after each key step (the first key after activation is occasionally lost).

## Acceptance criteria
- [ ] Demonstrated on the current app: a Mica screenshot in desktop mode, a click step (no ring on click), a held-key
      step; the forced-failure path still exits 1 with nothing saved.
- [ ] Never changes Windows settings; leaves no process running.

## Signals & storage
None.
