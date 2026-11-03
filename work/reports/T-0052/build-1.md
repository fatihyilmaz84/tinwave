HANDOFF T-0052 (round 1)
status: done
branch: t/T-0052-capture-tool-improvements   commit: cd3d5e8
summary: capture.ps1 now has -Mode window|desktop, click / key-down / key-up / wait steps, a readiness check after each step, cursor restore in `finally`, and a warning when the cursor can't be parked. Existing `-Keys "{TAB},{DOWN}"` still works. Only capture.ps1 changed. Demos ran against the main checkout's debug exe (D:\dev\Tinwave\target\debug\tinwave-desktop.exe, built 11:43) because the worktree has no target dir.

acceptance criteria:
- [x] Mica screenshot in desktop mode — d-mica.png (dark), d-light.png (light), 1804x1247 at 150 %, `-Mode desktop`. Desktop mode also captured the open native app menu (d-menu.png); window mode on the same steps (w-menu.png) shows no menu.
- [x] Click step, no ring on click — `-Keys "@click:113;160"` selects Albums with no focus ring (d-click.png); logical point converted with GetDpiForWindow (150 % → ×1.5).
- [x] Held-key step — `-Keys "{TAB},{TAB},{TAB},@down:DOWN"`: Albums selected with its ring while DOWN is held (k-held.png); plus `@wait:300,@up:DOWN` in desktop mode → k-released.png.
- [x] Forced-failure path: exit 1, nothing saved, nothing running, cursor back at (900,500) — `@click:5000;5000` fails with "step ... lies outside the window (1203 x 831 logical pixels at 150 % scaling)"; a scratch copy where the foreground can never be taken retries 5 times then fails with "could not be brought to the foreground"; bad steps (`@up:DOWN` without a down, `@wait:abc`, `@down:FOO`, `@click:-1;5`) are rejected before the app starts.
- [x] Never changes Windows settings; leaves no process running — only SetCursorPos, keybd_event, mouse_event and the app's own `--color-scheme`; tasklist after every run showed no tinwave-desktop.
- [x] Cursor restored at the end, also on failure — set to (900,500) before each run; read back (900,500) after success and failure; restore in `finally`.
- [x] Warning when the cursor can't be parked — scratch copy with a window rect covering everything printed "WARNING: the mouse pointer cannot be parked outside the window (the window covers every screen corner)..." and still saved the image.
- [x] Readiness after each key/click step — foreground, active window, keyboard focus (or an open menu owned by the window), Process.Responding, and two PrintWindow frames 150 ms apart identical; works with the native menu open; four consecutive `-Keys "{TAB}"` runs showed the ring on the first key.

Revert checks (scratch copies in scratchpad\t52): removing the key release in `finally` → SHIFT stayed down after `@down:SHIFT,@click:5000;5000` (released by hand; the real script releases it); removing the cursor restore → cursor stayed at the park corner (2559,0); unattainable foreground → abort path (exit 1, nothing saved).

checks: fmt pass, clippy n/a, tests n/a (no Rust changed; the tool verified by the runs above), deny pass
files changed: apps/tinwave-desktop/tools/capture.ps1
signals added/changed: none
ui: none (dev tool). Windows 11 Pro 26200, 150 %, OS dark. Screenshots in scratchpad\t52\: d-mica.png, d-light.png, d-click.png, d-menu.png, w-menu.png, k-held.png, k-released.png, w1.png (old `{TAB},{DOWN}` syntax).
docs updated: capture.ps1 header (modes, step tokens `@click:x;y`, `@down:KEY`, `@up:KEY`, `@wait:ms`, key names or hex VK; readiness; cursor handling; failure behaviour).
deviations from ticket: none. Notes: new tokens start with `@` (a SendKeys step starting with "@" is written `{@}`); held keys and the cursor released/restored in `finally`; foreground, readiness and parse checks fail the run (exit 1); the process is per-monitor-v2 DPI aware (SetProcessDpiAwarenessContext, fallback SetProcessDPIAware), image sizes unchanged on this monitor; Process.Responding used (WaitForInputIdle threw "process has no graphical interface"); a key the app receives and ignores can't be proven detected (header says so); on this build PrintWindow window mode produced the same Mica pixels as desktop mode (RGB 28,32,41 dark, 240,243,249 light; T-0036 found it solid), so the header says "the Mica backdrop only shows if the system composes it into the PrintWindow image"; native menus certainly need desktop mode.

proposals:
- P: apps/tinwave-desktop/README.md:28 — mention `-Mode desktop` and the click/key-down steps for capture.ps1 | size: S
- P: `@move:x;y` hover step to keep the cursor over a point for hover-state evidence | size: S

questions for lead: none

---
Lead (2026-11-03): P-0218 (README line) → lead applies on merge. P-0219 (hover step) parked → first UI ticket that needs
hover evidence.
