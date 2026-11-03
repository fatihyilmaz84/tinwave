---
name: lead
description: Run one Tinwave lead cycle — sync the board, triage proposals, groom tickets, dispatch Sonnet builders/spikes in worktrees, send finished work to the Opus reviewer, merge approved branches, and report to the PO. Use when the PO says "/lead", "run the team", "next cycle", or "continue the work".
---

# Lead cycle

You are the **lead** (run this session on Opus). Follow `work/README.md` exactly; it is the process contract.
You are the only writer of `work/`. The PO is the human in this chat.

## 0. Preconditions
- The repo must be a git repo with at least one commit on `main` (worktrees need it). If not, ask the PO.
- Read `work/ROADMAP.md`, `work/BOARD.md`, and any `decisions/` with an answer since last cycle.

## 1. Sync
- Collect finished agent results (builder HANDOFFs, REVIEWs, SPIKE RESULTs) from this session.
- Save each verbatim to `work/reports/T-NNNN/{build,review,spike}-N.md`.
- Save each proposal in them as `work/proposals/P-NNNN-slug.md` (template: `work/templates/proposal.md`).
  Next free number: highest existing + 1.

## 2. Triage proposals
For each untriaged proposal, set `outcome:` to one of `accepted → T-NNNN`, `merged → T-NNNN`, `parked`,
`rejected: <reason>`, `escalated → D-NNNN`. Escalation criteria are in `work/README.md`.

## 3. Groom
- Keep 3–6 tickets `ready`, following the epic order in `ROADMAP.md`.
- Use `/ticket` conventions (template `work/templates/ticket.md`). Enforce Definition of Ready; split anything L.
- Each ticket's `scope` must be precise enough to decide parallel safety.
- UI tickets: fill `## UI & design` from `docs/design/design-system.md` (section + canvas artboard). If the screen or
  pattern has no design reference, don't mark it ready: create a decision asking the PO for an artboard/direction.
  Adding a `Tw*` component or token that fits the principles is your call — make it its own small design ticket
  (scope `docs/design/` + `apps/tinwave-desktop/qml/Theme|Tw/`) that the screen tickets depend on.
- Keep `Theme.qml`/`qml/Tw/` and screen QML in separate tickets when possible so builders don't overlap scope.

## 4. Dispatch (max 3 agents in flight, including spikes and reviewers)
- Pick the highest-priority `ready` tickets whose `scope` does not overlap with anything `in_progress`/`in_review`.
- Start each with the Agent tool: `subagent_type: "tw-builder"` (or `"tw-spike"` for `type: spike`),
  `isolation: "worktree"`, a prompt containing: ticket path, branch name `t/T-NNNN-slug`, the round number,
  and any reviewer findings to address.
- Set ticket `status: in_progress`, `owner: <agent name>`, `branch:`.
- For a CHANGES_REQUESTED round, use SendMessage to the **same** builder instead of a new agent.

## 5. Review
- On a builder HANDOFF with `status: done`: set `in_review`, start `tw-reviewer` with ticket path, branch/worktree path,
  and the handoff text.
- `partial`/`blocked`: decide — answer the question and resume, split the ticket, or create a decision for the PO.

## 6. Integrate
- APPROVE: merge `t/T-NNNN-slug` into `main` (`git merge --no-ff`), run fmt/clippy/tests on `main`.
  Green → `status: done`, record merge commit, commit the work/ updates, then `git push origin main` (PO standing
  permission, work/README.md; never force-push — merge `origin/main` first if it moved). Red → revert the merge and
  reopen the ticket with the failure; don't push a red main.
- CHANGES_REQUESTED: round + 1 and back to the builder. After round 2 fails: split, take over yourself, or escalate.
- UI branches: the reviewer's `design check` must pass; skim the screenshots yourself before merging.
- Spikes: review the result yourself; turn the draft ADR into `docs/adr/NNNN-*.md` if accepted; create follow-up tickets.
  Escalate to the PO if the spike changes scope, license posture, or the spec.
- Push only `main`, only green, never force. Ticket branches stay local. Never delete branches with unmerged work.

## 7. Report
- Rewrite `work/BOARD.md` (template in the file itself).
- If an epic moved, a decision is needed, or the session is ending: write `work/digests/YYYY-MM-DD.md`
  (template below) and give the PO a short chat summary with links.

```
# Digest YYYY-MM-DD
## Done           T-NNNN title — one line on what it enables
## In flight      T-NNNN title — owner, status
## Needs the PO   D-NNNN question — your recommendation
## Risks          …
## Next cycle     what you will dispatch next
```

## Guardrails
- Don't write production code for a ticket a builder owns. You may fix trivial merge issues on `main`.
- Don't change scope or the spec without a PO decision.
- Be cost-aware: no more than 3 agents at once; prefer S/M tickets; don't re-dispatch a failing approach unchanged.
