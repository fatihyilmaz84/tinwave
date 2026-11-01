---
id: T-NNNN
title: <imperative, specific>
type: feature | bug | chore | spike
epic: E-NN
status: backlog            # backlog | ready | in_progress | in_review | changes_requested | done | blocked
priority: P1               # P0 now · P1 this epic · P2 later · P3 someday
size: S                    # S ≤ ~150 lines · M ≤ ~400 lines · L not allowed
depends_on: []
scope:                     # exact paths the builder may touch
  - crates/<crate>/
owner:                     # set by lead on dispatch
branch:                    # t/T-NNNN-slug
review_round: 0
created_by: lead           # lead | PO | from P-NNNN
---

## Context
Why this exists. Links: spec §…, docs/rules/…, docs/guides/…

## Scope
- What to build.

## Out of scope
- What not to touch, even if tempting (→ proposals).

## Acceptance criteria
- [ ] <testable statement> — verified by `<test or command>`
- [ ] (UI) design DoD in docs/design/design-system.md §8 met — verified by token grep, screenshots light + dark, keyboard walkthrough

## Signals & storage
Signals emitted (catalog rows), tiers, bytes/day — or "none".

## UI & design
<!-- UI tickets only; delete for non-UI work -->
- Design reference: docs/design/design-system.md §… and/or canvas artboard "<name>"
- Native controls: <Button, ComboBox, Switch, …>
- Tw components: <TwTrackRow, …> (new component/token? → name it and update design-system.md in this ticket)
- States to render: default, hover, pressed, focus, selected, disabled, loading/empty/unavailable — light + dark
- Platform notes: <anything that differs per OS, or "none">
- Keyboard & accessibility: <shortcuts, focus order, accessible names>

## Notes for the builder
Existing code to reuse, pitfalls, references.

<!-- spike tickets only -->
## Question
## Time box
## Success criteria
