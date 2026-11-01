# Tinwave team system

How Tinwave gets built: one **Opus lead** plans, reviews and coordinates; **Sonnet builders** implement;
an **Opus reviewer** checks every change; the **Product Owner** (human) sets direction and decides.
All work state lives in this `work/` folder as Markdown, in the repo, offline — like the product.

## Roles

| Role | Who | Owns | Never does |
|---|---|---|---|
| **Product Owner (PO)** | Human | Roadmap order, epic acceptance, scope/spec changes, privacy-tier and budget changes, decisions in `decisions/` | — |
| **Lead** | Main Claude Code session on **Opus** (runs the `/lead` skill) | Backlog, tickets, dispatching, triaging proposals, merging, `BOARD.md`, PO digests. **Only writer of `work/`.** | Write production code for a ticket a builder owns; change scope without the PO |
| **Builder** | `tw-builder` subagent on **Sonnet**, one per ticket, in its own git worktree | Implementing exactly one ticket: code, tests, docs touched by the ticket | Edit `work/`, touch files outside the ticket's scope, expand scope, merge |
| **Reviewer** | `tw-reviewer` subagent on **Opus**, read-only | Verdict on a builder's branch against the ticket + rules | Edit code, approve its own suggestions into scope |
| **Spike** | `tw-spike` subagent on **Sonnet**, own worktree | Time-boxed research/prototype answering one question; findings + draft ADR | Ship production code |

Why this split: Opus is spent where judgment matters (decomposition, review, triage, architecture);
Sonnet does the volume of implementation cheaply and fast; isolated worktrees let builders run in parallel.

## Folder layout

```
work/
  README.md          this file
  ROADMAP.md         epic order — PO-owned
  BOARD.md           current state of every open ticket — lead-maintained
  epics/             E-NN-slug.md
  tickets/           T-NNNN-slug.md
  proposals/         P-NNNN-slug.md   ideas from builders/reviewers/spikes awaiting triage
  decisions/         D-NNNN-slug.md   questions for the PO
  reports/T-NNNN/    build-N.md, review-N.md  (saved by the lead from agent handoffs)
  digests/           YYYY-MM-DD.md    lead → PO summaries
  templates/         ticket, epic, proposal, decision, build report, review report
```

## Ticket lifecycle

```
backlog ──groom──> ready ──dispatch──> in_progress ──handoff──> in_review ──APPROVE──> done
                     ^                     ^                        │
                     │                     └──CHANGES_REQUESTED─────┘  (max 2 rounds, then lead
   blocked <── any state when a dependency / PO decision is missing       splits, takes over, or escalates)
```

### Definition of Ready (lead checks before `ready`)
- Context links to the spec section, rules and guides it depends on.
- Acceptance criteria are concrete and testable (a builder can tick each one by running something).
- Size **S** (≤ ~150 changed lines) or **M** (≤ ~400). **L is not allowed** — split it.
- `scope` lists the crates/paths the builder may touch; dependencies are `done`.
- Signals and storage impact stated (every playback/UI feature emits signals — see the signal catalog).
- **UI tickets:** the `## UI & design` section names the design reference (design-system.md section and/or
  canvas artboard), the native controls and `Tw*` components used, required states, and any new token/component.
  A screen or pattern with no design reference is not ready — get one from the PO first.
- No open PO decision blocks it.

### Definition of Done (reviewer checks, lead confirms)
- Every acceptance criterion verified, with the command or test that proves it.
- `cargo fmt --check`, `clippy -D warnings`, tests, `cargo deny` pass on the branch **and again on main after merge**.
- Tests per [testing.md](../docs/rules/testing.md), including signal emission/interpretation/privacy tests when signals are touched.
- Docs, signal catalog, ADRs updated when the change affects them. No `TODO`s, placeholders or silent fallbacks.
- Storage estimate added for any new signal; growth test still passes.
- **UI changes:** the design Definition of Done in [design-system.md §8](../docs/design/design-system.md) holds
  (tokens only, native controls, light + dark, keyboard + accessible names, signals, screenshots in the handoff).

## The lead cycle (`/lead`)

1. **Sync** — read `BOARD.md`, new proposals, answered decisions, finished agent handoffs.
2. **Triage proposals** — accept (→ ticket), merge into an existing ticket, park in backlog, reject with a reason,
   or escalate to the PO as a decision. Every proposal gets an outcome line.
3. **Groom** — keep 3–6 tickets `ready`, in roadmap order.
4. **Dispatch** — start up to **3 builders in parallel**, only for tickets whose `scope` does not overlap.
   Spikes count toward the limit.
5. **Review** — when a builder hands off: save `reports/T-NNNN/build-N.md`, start a reviewer on the branch.
6. **Integrate** — on APPROVE: merge the branch into `main` locally, run full checks on `main`, mark `done`.
   On CHANGES_REQUESTED: send findings back to the **same** builder (keeps its context). After round 2: split,
   take over, or escalate.
7. **Report** — update `BOARD.md`; write a digest for the PO when an epic moves, a decision is needed, or
   at the end of a working session.

## Escalate to the PO (create a `decisions/` entry) when

- Scope differs from the spec, or a spec section must change.
- A new T2/T3 signal, a new OS permission, or a change to the storage budget.
- A dependency with a copyleft/commercial license concern (GPL, LGPL static linking, paid SDKs).
- User-visible behaviour not described in the spec or guides — including a new screen or UI pattern with no
  reference in the [design system](../docs/design/design-system.md) or the PO's design canvas.
- A change to a design principle, the native-first stance, the color/type tokens' look, or the Qt version floor.
- An epic is ready for acceptance (PO demo).
- Cost: more than 3 parallel agents, or a ticket that failed review twice.

Everything else (technical choices inside the rules, ticket ordering within an epic, accepting small proposals,
adding a `Tw*` component or token that fits the existing design principles) the lead decides and records in the
ticket, design-system.md, or an ADR.

## Git rules

- `main` is always green. One branch per ticket: `t/T-NNNN-slug`, in a worktree.
- The PO grants the lead standing permission to **commit and merge locally**. **Never push** or create remote PRs
  without the PO asking.
- Commit messages: `T-NNNN: <summary>`.

## Proposals: how builders suggest work

Builders and reviewers do not create tickets. They put proposals in their handoff (title, why, rough size,
evidence). The lead saves each as `proposals/P-NNNN-*.md` and triages it in the next cycle. Good proposals:
a bug seen nearby, a missing test, a refactor that would unblock the next ticket, a risk, a cheaper approach,
a new signal idea (with its tier and bytes/day).
