REVIEW T-0035 (round 2)
verdict: CHANGES_REQUESTED
checks (run by reviewer): fmt pass (exit 0), clippy pass (`-D warnings`, exit 0), tests pass (nextest: 402 run, 402 passed, 6 skipped), deny pass (advisories, bans, licenses and sources ok)

acceptance criteria:
- [x] The self-test passes for 15 accents × 2 schemes on every surface (≥ 4.5:1 text, ≥ 3:1 UI glyphs) — verified by: my harness outside the worktree (copy of the branch Theme.qml), run with `qml -I . check.qml`:
  - `Theme.checkAccents` deep-equals the T-0011 list (n=15).
  - `Theme.selfCheck()` with no arguments printed `rows=30 failures=0`.
  - `selfCheck(explicit list)` gave identical rows.
- [x] design-system.md §3 updated; changed look noted for the PO — verified by: diff 727589c..4683dd8.
  - §3.1 contrast rule and the §5 states line both say `hover` is never composited over `selected`, and the `surfacesOf` comment (Theme.qml:92-93) says the same. This closes the round-1 blocking finding per the lead's option (a).
  - §3.1 lists the 15 hexes and the no-argument `selfCheck()`.
  - No token values changed, so there are no new look changes.
- [x] P-0116 shell tokens — unchanged from round 1. Main.qml now goes through `firstStartWidth/Height` (see the blocking finding).

blocking findings:
- apps/tinwave-desktop/qml/Main.qml:15-16 — the first-start clamp is a live binding (`width: Theme.firstStartWidth(Screen.desktopAvailableWidth)`), so it is not evaluated only once at first start.
  - Why it fires: a user resize does not break a QML binding on Window `width`/`height`. Whenever `Screen.desktopGeometryChanged` fires, the binding re-evaluates and resizes the window back to the clamped default. That happens on monitor attach or detach, a resolution or scale change, or the window's screen changing.
  - Failure scenario: the user enlarges Tinwave to 1600 × 1000, then docks the laptop or plugs in a monitor, and the window jumps back to 1200 × 800. It would also fight the planned "last size is remembered" restore.
  - Proof: a probe with the same binding shape, fed through a property (scratchpad\rev35r2\rebind.qml, external resize via Win32 SetWindowPos in resize.ps1). After the external resize the window was 2085 × 1363; after the available-size input changed it was 1200 × 800.
  - Round 1 bound constant tokens, so this was introduced in round 2.
  - Suggested fix (S): keep `width: Theme.windowDefaultWidth` / `height: Theme.windowDefaultHeight` (or no binding), and assign the clamped size imperatively once in the existing `Component.onCompleted`. Re-check that the window still opens at 1200 × 800 here.

design check: pass, apart from the blocking finding above, which is behavioural rather than visual.
- Token grep in qml/ outside Theme/ clean; only `spacing: 0` (Main.qml:38) and `currentIndex: 0` (Sidebar.qml:63).
- Focus-ring comment (Theme.qml:323) matches §5 (`accentText`).
- Clamp maths correct, never below the minimum: 2560×1392 → 1200×800; 1366×720 → 1200×672; 1280×672 → 1200×624; 1093×566 → 1093×560 (minimum height; by design); 0×0 → 880×560.
- The real app opens normally on this machine: client area 1200×800 logical, window 1215×837, title "Tinwave".
- Every process started was killed. No Windows settings were changed.

non-blocking findings:
- Main.qml:15-16 and design-system.md §3.3 — `Screen.desktopAvailableWidth/Height` is the whole virtual desktop, not the window's screen. A 1366×768 laptop next to a 1080p monitor gets no height clamp. QML exposes no per-screen available geometry; say "virtual desktop" in §3.3 or accept the limitation.
- design-system.md §3.3 — "clamped to the available screen minus `windowChromeAllowance` 48" reads as if both dimensions subtract 48; only the height does.
- build-2.md says 1366×768 @ 100 % gets "about 1200 x 720"; with a 48 px taskbar it is 1200×672 client. Behaviour fine; handoff number off.

proposals:
- none new (P-0133 canvas sync is already tracked)

notes for lead: scope clean (Theme.qml, Main.qml, Sidebar.qml, design-system.md only). A round 3 is the one-line change from a binding to a one-time assignment; check the window opens at 1200×800 again after it. Harness and output: scratchpad\rev35r2 (check.qml, err.txt, rebind.qml, resize.ps1, err2.txt).

---
Lead (2026-11-02): round 2 failed on a two-line fix exactly as specified → the lead took it over (work/README.md:
after round 2, split / take over / escalate). Main.qml binds the constant default tokens and assigns
`firstStartWidth/Height(Screen.desktopAvailable*)` once in `Component.onCompleted`; §3.3 now says "clamped once at start
to the available virtual-desktop size, the height also minus `windowChromeAllowance`". Verified: desktop tests 4 unit +
2 startup pass; launched app client area 1200 × 800 logical, no QML warnings, process killed.
