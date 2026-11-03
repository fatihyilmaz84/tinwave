REVIEW T-0046 (round 1)
verdict: APPROVE
checks (run by reviewer): fmt pass, clippy skipped (no code changed, per lead), tests skipped (no code changed, per lead), deny pass
Single commit 766d958; only 6 files under `docs/design/canvas/` (34 lines). `git merge-tree --write-tree main t/T-0046-canvas-token-sync` clean (tree fa457e67); worktree clean.

acceptance criteria:
- [x] No old hex remains; old → new table per file — case-insensitive listing of every 3-, 6-, 8-digit hex in all canvas files: none of #6E6E77, #E6F1F2, #2E7D46, #9A5B00, #8C8C95, #1B2B2D remains. The handoff table matches the diff counts exactly: #8C8C95 → #94949C ×10; #6E6E77 → #64646D ×10; #E6F1F2 → #E7F1F2 ×6; #2E7D46 → #2A7240 ×4; #9A5B00 → #925700 ×2; #1B2B2D → #1E2B2E ×2.
- [x] Canvas files still render — builder's headless-Edge renders: Main-after.png (swatch sheet with the new values, intact layout); Library-Windows-after.png (20 px nav glyphs clean in 34 px rows).
- [x] P-0158 — the only non-colour edits are four `<svg width="18" height="18"` → `"20"` (Library-Windows.dc.html:27, :36; SignalCenter-Windows.dc.html:27, :36); Theme.qml:331 `iconSize: 20`; §3.4 says 20 px.

Value correctness: every new value equals Theme.qml:160-163 and §3.1 lines 78-89 (light/dark textFaint #64646D/#94949C, srcDownload #2A7240, warning #925700). `selected` recomputed by hand: light Tin Teal #0B6E78 at 10 % over #FFFFFF = (230.6, 240.5, 241.5) → #E7F1F2; dark #5CC6CF at 12 % over #161618 = (30.4, 43.1, 46.0) → #1E2B2E. Main.dc.html:197-209 swatch sheet checked line by line: all 30 light/dark swatches equal §3.1 / Theme.qml (incl. accent fallbacks #0B6E78/#5CC6CF, onAccent defaults #FFFFFF/#0F2F33). Alpha order correct: hover #0000000A / #FFFFFF0F and separator #0000001A / #FFFFFF14 in CSS #RRGGBBAA order = QML #0A000000 / #0FFFFFFF / #1A000000 / #14FFFFFF; other 8-digit values are illustrations. Scope respected (word-level diff: only hex values and the four icon sizes).

blocking findings:
- none

design check: pass (docs only; canvas agrees with §3.1 colours and §3.4 icon size).

non-blocking findings:
- Main.dc.html:197-209 — the swatch sheet has no `danger` swatch (#B42318 / #F2877D, §3.1:90); predates the branch; could go with T-0055.
- Near-miss surfaces (e.g. Library-Windows.dc.html:25 sidebar #EDEFF3 vs token #ECECEF) — already P-0211 → D-0008.

proposals:
- none new

notes for lead: safe to merge; docs only, can't conflict with code tickets. Nothing left running.

---
Lead (2026-11-03): merge. `danger` swatch added to T-0055's scope.
