---
id: T-0002
title: "Spike: CXX-Qt + QML virtualized list — viability, memory, startup"
type: spike
epic: E-00
status: done
priority: P0
size: M
depends_on: []
scope:
  - spikes/T-0002-cxx-qt/
owner: tw-spike (round 1)
branch: t/T-0002-spike-cxx-qt
review_round: 0
created_by: lead
---

## Context
Spec §2 and §16 Phase 0 require validating Rust + Qt Quick/QML and the bridge. Rules: docs/rules/qml-ui.md.
Memory targets: spec §12 (idle 50–100 MB, browsing 80–160 MB).

## Machine (lead grooming 2026-11-02)
- Qt **6.12.0** MSVC kit: `C:\Qt\6.12.0\msvc2022_64` (`qmake.exe`, `qml.exe`, Qt Quick Controls styles incl.
  FluentWinUI3, Fusion, Native). Set `QMAKE=C:\Qt\6.12.0\msvc2022_64\bin\qmake.exe`; add its `bin` to PATH to run.
  The old `C:\Qt\6.10.1\mingw_64` kit can't link with MSVC Rust — don't use it.
- Ninja and CMake: `C:\Qt\Tools\Ninja`, `C:\Qt\Tools\CMake_64` (also CMake 4.0.2 on PATH).
- Rust 1.92 stable x86_64-pc-windows-msvc; Visual Studio 2026 Community, MSVC 14.50 (ABI-compatible with the
  msvc2022 kit); Windows SDK 10.0.26100.
- The design system's floor is Qt **6.8 LTS**; only 6.12 is installed. Measure on 6.12 and state which findings
  depend on > 6.8 (from Qt docs/changelogs); a floor change is an ADR + PO decision.

## Question
Is CXX-Qt a viable bridge for Tinwave on Windows, and what memory/startup cost does a minimal Qt Quick app have?

## Scope
- Standalone prototype: QML window with a `ListView` bound to a Rust `QAbstractListModel` serving 100 000
  synthetic rows lazily (paged fetch), plus one async image provider loading a thumbnail.
- Rust → Qt thread result posting (simulate a 200 ms Rust task without freezing the UI).
- Measure on Windows: cold and warm startup to first frame, idle RSS, RSS while fast-scrolling, binary + Qt deploy size.
- Note build complexity: CMake vs Cargo-driven build, Qt version, MSVC requirements.
- Compare briefly with one alternative (e.g. `cxx` + hand-written C++ QObjects) on effort, not full implementation.

## Time box
1 working session.

## Success criteria
- Recommendation with measurements, build instructions, and a draft ADR.

## Acceptance criteria
- [ ] SPIKE RESULT with measured numbers and the machine described.
- [ ] Draft ADR at `spikes/T-0002-cxx-qt/ADR-draft.md`.
- [ ] Licenses of Qt modules and CXX-Qt recorded (LGPL Qt dynamic linking implications noted).

## Signals & storage
None. (Note in findings whether QML hover/scroll events can be cheaply forwarded to Rust for UI counters.)

## Outcome
2026-11-02: CXX-Qt 0.10, Cargo-driven, dynamic Qt → ADR 0008 (merge f5ebab3). Follow-ups T-0040..T-0042.
