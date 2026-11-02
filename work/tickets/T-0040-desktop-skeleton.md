---
id: T-0040
title: apps/tinwave-desktop skeleton — CXX-Qt crate, FluentWinUI3 window shell, Theme.qml, start-up error path
type: feature
epic: E-01
status: backlog
priority: P0
size: M
depends_on: []
scope:
  - apps/tinwave-desktop/
  - Cargo.toml            # add "apps/*" to members (T-0001 note) + [workspace.dependencies]
  - Cargo.lock
owner:
branch:
review_round: 0
created_by: from T-0002 (ADR 0008), T-0011 (ADR 0007)
---

## Context
ADR 0008 (CXX-Qt 0.10, Cargo-driven: build.rs with `.crate_include_root(None)`, `objectCreationFailed` exit path,
deny(unsafe_code) + allow on bridge modules), ADR 0007 / design-system §2–§4 (FluentWinUI3 selected explicitly,
layout: native title bar, sidebar 232, content, transport bar 80), prototypes in spikes/T-0002-cxx-qt/ and
spikes/T-0011-native-styles/ (Theme.qml). architecture.md: thin view models, one composition root `app/wiring.rs`.

## Scope
- New crate `apps/tinwave-desktop` (binary) with build.rs, `main.rs`, `app/wiring.rs` (empty composition root for now),
  QML module with `Main.qml` (window shell: sidebar with the §4 groups as native ItemDelegates, empty content area with
  `TwEmptyState`-style text, transport bar placeholder area drawn only with tokens) and `Theme/Theme.qml` ported from
  the T-0011 prototype (T-0035 token fixes land separately).
- Style selection at start-up (FluentWinUI3 on Windows; Fusion elsewhere), light/dark following the OS.
- QML load failure exits with a non-zero code and a clear message (never hangs).
- `cargo run -p tinwave-desktop` opens the window; workspace `members` gains `"apps/*"`.

## UI & design
- Design reference: design-system.md §2, §3, §4 (layout) and canvas "Library · Windows" (shell only).
- Native controls: ApplicationWindow, ItemDelegate, TextField (search), ToolButton. Tw components: none yet.
- States: light + dark; keyboard focus order sidebar → content; accessible names on every interactive item.

## Acceptance criteria
- [ ] Window opens in < 1 s warm on the dev machine; idle private working set reported (target ≤ 60 MB for the shell).
- [ ] Token grep: no raw colours/sizes in QML outside `Theme/` (design-system §8).
- [ ] QML load error ⇒ exit code ≠ 0 with a message — test.
- [ ] Screenshots light + dark; keyboard walkthrough noted.
- [ ] fmt, clippy `-D warnings`, tests, `cargo deny check` pass; generated bridge code clippy-clean.

## Signals & storage
None yet (view models and signals arrive with the screen tickets).
