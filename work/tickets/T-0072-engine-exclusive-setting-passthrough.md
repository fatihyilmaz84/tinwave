---
id: T-0072
title: Replace the controller's SharedOutput wrapper with an engine passthrough for the exclusive setting
type: chore
epic: E-01
status: backlog
priority: P2
size: S
depends_on: [T-0066, T-0067]
scope:
  - crates/tinwave-playback/src/engine.rs        # wants_exclusive passthrough only
  - crates/tinwave-playback/src/controller/       # remove setting.rs wrapper
owner:
branch:
review_round: 0
created_by: from P-0284 (T-0067 build-1)
---

## Context
T-0067 reads `AudioOutput::wants_exclusive()` through `controller/setting.rs` (`SharedOutput`, a Mutex-wrapped output
that must forward every trait method), to avoid editing engine.rs while T-0066 did.

## Scope
- `Engine::wants_exclusive()` forwards to its output; the controller uses it; `setting.rs` and the wrapper go away.

Until this lands: any default method added to `AudioOutput` must be forwarded by `SharedOutput` (setting.rs);
nothing fails to compile if it isn't (T-0067 review-1).

## Acceptance criteria
- [ ] All T-0067 controller_exclusive / controller_signals tests pass unchanged.
- [ ] No Mutex around the output; fmt, clippy, nextest, deny pass.

## Signals & storage
None.
