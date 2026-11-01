---
name: ticket
description: Create or refine a Tinwave ticket, epic, or PO decision in work/ from a short description, following the Definition of Ready. Use when the PO or lead says "/ticket", "make a ticket for…", "add to the backlog", or "split T-NNNN".
---

# Ticket authoring

1. Pick the type: `feature`, `bug`, `chore`, `spike` (question + time box), or an epic / decision.
2. Next free id: highest `T-NNNN` in `work/tickets/` + 1 (`E-NN` for epics, `D-NNNN` for decisions).
3. Copy the template from `work/templates/` and fill every field. File name: `T-NNNN-short-slug.md`.
4. Ground it: link the spec section, rules and guides. Read the relevant code if any exists, and name real
   files/functions to reuse.
5. Acceptance criteria: each one testable by a command or a named test. Include signal emission and
   privacy-off criteria for anything that touches playback or UI. Include storage criteria for new signals.
   For UI: fill `## UI & design` (design-system.md section / canvas artboard, native controls, `Tw*` components,
   states, platform and keyboard notes) and add the design-DoD criterion. No design reference → decision for the PO.
6. Size: S (≤ ~150 lines) or M (≤ ~400). If it's bigger, split into several tickets with `depends_on`.
7. `scope`: the exact crates/paths the builder may touch. Narrow scope = safe parallelism.
8. Status `backlog` unless Definition of Ready (in `work/README.md`) is fully met — then `ready`.
9. Add it to `work/BOARD.md`.

When splitting a ticket: keep the original id for the first slice, create new ids for the rest, update `depends_on`.
When the PO asks for something that changes scope or the spec, create a decision instead (template `decision.md`)
with options and your recommendation.
