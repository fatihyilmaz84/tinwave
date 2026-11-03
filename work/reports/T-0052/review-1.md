REVIEW T-0052 (round 1)
verdict: CHANGES_REQUESTED
checks (run by reviewer): fmt pass, clippy n/a (no Rust changed), tests n/a (tool run by hand, below), deny pass. `git merge-tree --write-tree main t/T-0052-capture-tool-improvements` clean (d854339…). Diff touches only apps/tinwave-desktop/tools/capture.ps1; nothing under work/.

All runs: worktree script against D:\dev\Tinwave\target\debug\tinwave-desktop.exe (11:43; later main commits don't touch the UI), via a per-monitor-DPI-aware wrapper that set the cursor to (900,500) first and afterwards read the cursor, the async state of SHIFT/CTRL/ALT/DOWN/TAB/SPACE, and the tinwave-desktop count.

acceptance criteria:
- [x] Mica screenshot in desktop mode — `-Mode desktop`, dark, `{TAB},@down:SPACE,@up:SPACE` → d-menu.png 1804x1247 with the composed window and the open native app menu ("Tinwave 0.1.0 / Quit"); `@click:30;56,@wait:500` (click-menu.png) also opens it.
- [x] Click step, no ring — `-Keys "@click:80;196"` (w-click.png) selects Artists, no ring, no hover fill; scaling right.
- [x] Held-key step — `{TAB},{TAB},{TAB},@down:DOWN` (k-held.png): Albums with its ring while held; `...,@wait:300,@up:DOWN` desktop (k-released.png) same; no key stuck. `{TAB},{TAB},@down:SHIFT,{TAB}` (shift-sendkeys.png) worked as Shift+Tab.
- [x] Old syntax `-Keys "{TAB},{DOWN}"` — w-old.png, ring on the menu button, exit 0.
- [x] Forced-failure path — `@click:5000;5000`, `@down:SHIFT,@click:5000;5000`, `{TAB},@down:DOWN,@click:1;5000`: exit 1, nothing saved, cursor back, no key stuck, no process left. Bad steps exit 1 in 0 s before the app starts: `@up:DOWN`, `@wait:abc`, `@wait:0`, `@wait:99999999999`, `@down:FOO`, `@click:-1;5`, `@click:5`, `@foo:1`, `@down:`, a second `@up:DOWN`.
- [x] Never changes Windows settings — only SetCursorPos, keybd_event, mouse_event, the existing foreground calls, and process-local SetProcessDpiAwarenessContext(-4) (line 108); no SystemParametersInfo or registry.
- [x] No process left — 0 after every run once the kill completes (sometimes 1 right after return: Stop-Process not awaited).
- [x] Cursor restored, also on failure — 12 later runs read back the start position for DPI-aware and -unaware observers, on success, runtime and parse failure; restore in `finally` (capture.ps1:370).
- [x] Warning when the cursor can't be parked — scratch mutant printed it, still saved, restored the cursor; deleted.
- [x] Readiness bounded — Get-Foreground 5 attempts; focus/active wait 3 s; Responding 3 s; frame stability 20 × 150 ms; startup window wait 80 × 250 ms; menu-open case via hwndMenuOwner.

blocking findings:
- capture.ps1:5 — the header example `-Mode desktop -Keys "@click:24;24,@wait:500" -Out menu-dark.png` doesn't open a menu: click points count from the frame top-left, title bar included; 24;24 logical = 36,36 physical, in the title bar. Run as written it saved hdr-example.png with no menu, exit 0. Fix: `@click:30;56,@wait:500` (the menu button; verified in click-menu.png).

design check: n/a

non-blocking findings:
- capture.ps1:25 — "in device-independent pixels, as QML sees them" reads like QML scene coordinates; the origin is the frame top-left including the title bar (QML y = click y − ~31 logical px here). Say they're image coordinates divided by the scale.
- capture.ps1:369 — `Stop-Process -Force` not awaited; add `$proc.WaitForExit(5000)`.
- capture.ps1:130 — a plain space SendKeys step (" ") is trimmed away (pre-existing); mention `@down:SPACE,@up:SPACE` in the header.

proposals:
- P: WaitForExit with a bound after Stop-Process | size: S

notes for lead: fix is one header line (plus optional wording); everything else checks out. One first run left the cursor at (597,893) — not reproduced in 12 later runs (likely mouse moved during the run); not counted. Nothing left running; no keys held. Evidence in scratchpad\r52\ (d-menu, click-menu, hdr-example, w-click, k-held, k-released, shift-sendkeys, w-old).

---
Lead (2026-11-03): round 2 = the header example (blocking) + coordinate wording + WaitForExit (P-0225 merged here) +
the space note.
