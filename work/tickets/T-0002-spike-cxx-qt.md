---
id: T-0002
title: "Spike: CXX-Qt + QML virtualized list — viability, memory, startup"
type: spike
epic: E-00
status: ready
priority: P0
size: M
depends_on: []
scope:
  - spikes/T-0002-cxx-qt/
owner:
branch:
review_round: 0
created_by: lead
---

## Context
Spec §2 and §16 Phase 0 require validating Rust + Qt Quick/QML and the bridge. Rules: docs/rules/qml-ui.md.
Memory targets: spec §12 (idle 50–100 MB, browsing 80–160 MB).

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
