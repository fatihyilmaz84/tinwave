HANDOFF T-0052 (round 2)
status: done
branch: t/T-0052-capture-tool-improvements   commit: e1018f7
summary: Review findings fixed: the header click example uses `@click:30;56` (run as written: opens the menu); click
docs state image-coordinate semantics; the space bar is documented; `finally` waits (bounded) for the app to exit. Only
capture.ps1 changed.

acceptance criteria:
- [x] Header example 5 as written (`-Mode desktop -Keys "@click:30;56,@wait:500"`, plus `-Exe`) opens the app menu —
      menu-dark.png shows "Tinwave 0.1.0 / Quit", exit 0, cursor restored to (900,500).
- [x] Click coordinates documented (header 22-26): image (frame) pixels / (DPI/96), origin at the frame top-left incl.
      the native title bar, not QML scene coordinates; worked example image (45,84) = `@click:30;56` at 150 %.
- [x] `$proc.WaitForExit(5000)` after `Stop-Process -Force` in `finally`; tasklist after every run shows no
      tinwave-desktop.
- [x] Space bar: steps are trimmed, so use `@down:SPACE,@up:SPACE`; `-Mode desktop -Keys "{TAB},@down:SPACE,@up:SPACE,@wait:500"`
      opens the same native menu (space-menu-desktop.png); window mode doesn't show it (space-menu.png), as expected.

checks: fmt pass, clippy n/a, tests n/a, deny pass
files changed: apps/tinwave-desktop/tools/capture.ps1
screenshots: scratchpad\t52\menu-dark.png, space-menu-desktop.png, space-menu.png
proposals: none new
questions for lead: none

---
Lead (2026-11-03): round-2 diff (17 lines: header example, coordinate wording, space note, bounded WaitForExit)
checked by the lead against review-1 and menu-dark.png (menu open) instead of a second review round — the reviewer had
called the fix one header line. Merged; README row updated (P-0218).
