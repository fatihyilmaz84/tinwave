---
id: T-0044
title: Artwork image provider in apps/tinwave-desktop — T-0041 lifetime pattern, RAII UI owner, teardown stress check
type: feature
epic: E-01
status: in_progress
priority: P0
size: M
depends_on: [T-0040]
scope:
  - apps/tinwave-desktop/src/app/       # UI owner (engine + providers), wiring
  - apps/tinwave-desktop/src/image/     # provider bridge (Rust side)
  - apps/tinwave-desktop/cpp/           # the C++ response/task shim
  - apps/tinwave-desktop/build.rs
  - apps/tinwave-desktop/tools/stress/  # dev-only teardown stress target
  - apps/tinwave-desktop/tests/
owner: tw-builder
branch: t/T-0044-artwork-image-provider
review_round: 2
created_by: from T-0041 (P-0122)
---

## Context
ADR 0008 decision 5 and its "image provider lifetime" addendum; docs/rules/qml-ui.md (image providers);
docs/rules/testing.md (bridge lifetime). Verified reference implementation: spikes/T-0002-cxx-qt/cpp/, src/image.rs,
src/main.rs (T-0041 "prod" pattern). No real artwork source exists yet: the provider serves thumbnails from an
injected `ArtworkSource` trait (a fake source in tests; the library-backed source arrives with the library tickets).

## Scope
- Port the provider with the six-rule pattern; bounded pool, newest-first, closing flag, bounded LRU (size from a const
  with a comment; spec §12 memory budget).
- A UI owner struct that owns the engine and every provider; its Drop quiesces all providers, then drops the engine;
  `main` also calls the explicit shutdown after `exec()`.
- Dev-only stress target (QML + script) equivalent to crash-T-0041/run-batch.ps1 for the app's provider.

## Acceptance criteria
- [ ] Test: dropping the UI owner quiesces providers before the engine (order asserted).
- [ ] Headless test: cancel storm + quit with tasks in flight exits cleanly (offscreen platform).
- [ ] Stress target: >= 300 Release runs, 0 crashes, 0 exit hangs — numbers in the handoff.
- [ ] No pointer to a response leaves the reader thread (review check); exactly one finished() per response (test).
- [ ] fmt, clippy -D warnings, tests, deny pass.

## Signals & storage
None (artwork display is not a preference interaction).
