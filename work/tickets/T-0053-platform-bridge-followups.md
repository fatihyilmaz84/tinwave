---
id: T-0053
title: Platform bridge follow-ups — portable winId, same-process window guard, menu re-flush, tighter probe test
type: chore
epic: E-01
status: ready
priority: P2
size: S
depends_on: [T-0036]
scope:
  - apps/tinwave-desktop/src/platform/
  - apps/tinwave-desktop/src/app/platform_bridge.rs
  - apps/tinwave-desktop/cpp/                 # the winId shim only
  - apps/tinwave-desktop/build.rs             # registering the shim only
  - apps/tinwave-desktop/tests/platform_qml.rs
owner:
branch:
review_round: 0
created_by: from T-0036 review-1 (P-0194..P-0196)
---

## Context
work/reports/T-0036/review-1.md, non-blocking findings and proposals.

## Scope
- `winId` through a one-line C++ shim returning `rust::usize` (cxx's `usize` is `size_t`, WId is `quint64`; differs
  outside MSVC), so the bridge can compile on Linux/macOS (T-0038/T-0039).
- `live_window` accepts only a window owned by this process (`GetWindowThreadProcessId` == current) and the bridge
  attaches only on the `windows` QPA (offscreen winIds are counters, not HWNDs); fix its doc comment.
- Re-apply the menu scheme (FlushMenuThemes) when an OS colour-scheme change arrives while the app follows "system".
- `uxtheme` function pointers resolved once (`OnceLock`), no load reference per call.
- platform_qml: `durationFast == (reducedMotion ? 0 : 120)` style exact assertions.

## Acceptance criteria
- [ ] Unit tests for the ownership guard (a foreign-process window is refused: e.g. the shell's desktop window) and for
      the re-flush on a synthetic scheme change.
- [ ] Offscreen tests record "not attached (QPA offscreen)" instead of "IsWindow failed".
- [ ] fmt, clippy -D warnings, tests, deny pass; no Windows setting changed.

## Signals & storage
None.
