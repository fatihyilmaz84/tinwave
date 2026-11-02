HANDOFF T-0035 (round 1)
status: done
branch: t/T-0035-design-token-fixes   commit: 727589c
summary: Fixed the §3.1 contrast bugs in Theme.qml and design-system.md. The self-check now passes for 15 accents x 2 schemes on every surface, including hovered and selected rows. Added the five shell tokens (default window size, hairline, content radius, sidebar row height), used them in Main.qml and Sidebar.qml, and replaced the ToolSeparator with a `separator`-token hairline. Most of the prototype's accentFill, computed onAccent and rgba-alpha work was already in the ported Theme.qml. The remaining work was the actual token values, a repeatable self-check, `selected` back-off, an onAccent fix, and the docs.

acceptance criteria:
- [x] Self-test passes for 15 accents x 2 schemes on every surface (text >= 4.5:1, UI glyphs >= 3:1) — proof: `qml -I . check.qml` (a scratch harness, not committed) calling the new `Theme.selfCheck(accents)` printed `SELFCHECK failures=0`. All 30 rows are ok. Sanity check: with the old textFaint/srcDownload hexes restored the same harness reports FAIL (textFaint 3.94 on hover/sidebar).
- [x] design-system.md §3 updated; changed look noted for the PO — proof: §3.1 rewritten (token table, contrast rule, check, worst cases), §3.2 note, §3.3 sizes/radius, §5 focus-ring wording, §7. Look changes are listed under "deviations" below and under questions for the lead.

Contrast of every fixed pair, final values. The worst surface is hover/sidebar in light and hover/raised in dark. "selected" is Tin Teal 10 %/12 %.
- light text #1B1B1F: window 15.49, sidebar 14.56, content/raised 17.17, selected 14.83, worst 13.40
- light textMuted #5D5D66: 5.88 / 5.53 / 6.52 / 5.63, worst 5.09
- light textFaint #6E6E77 -> #64646D (was 4.28 on sidebar, 4.39 on selected): 5.29 / 4.97 / 5.86 / 5.06, worst 4.57
- light srcServer #1F5FA8: 5.81 / 5.46 / 6.44 / 5.56, worst 5.03
- light srcDownload #2E7D46 -> #2A7240 (was 4.30 on sidebar): 5.29 / 4.98 / 5.87 / 5.07, worst 4.58
- light warning #9A5B00 -> #925700 (was 4.24 on hover/sidebar): 5.29 / 4.97 / 5.86 / 5.06, worst 4.57
- light danger #B42318: 5.93 / 5.58 / 6.57 / 5.68, worst 5.13
- dark text #F2F2F4: 16.16 / 15.04 / 16.16 (content) / 14.53 (raised) / 13.04 (selected), worst 12.28
- dark textMuted #A6A6AF: 7.48 / 6.96 / 7.48 / 6.73 / 6.04, worst 5.68
- dark textFaint #8C8C95 -> #94949C (was 4.12 on hover/raised): 6.00 / 5.59 / 6.00 / 5.40 / 4.84, worst 4.56
- dark srcServer #7BB0EE: 7.99 / 7.44 / 7.99 / 7.19 / 6.45, worst 6.07
- dark srcDownload #76C48E: 8.65 / 8.05 / 8.65 / 7.78 / 6.98, worst 6.57
- dark warning #E0A453: 8.27 / 7.69 / 8.27 / 7.43 / 6.67, worst 6.28
- dark danger #F2877D: 7.35 / 6.84 / 7.35 / 6.61 / 5.93, worst 5.58
- Across the 15 accents the minimum of every text token on every surface is light 4.57 and dark 4.50. The dark minimum is textFaint on `selected` for the white accent, where the back-off clamps to 4.50.
- accentText, minimum over all surfaces: light 4.50 to 16.40, dark 4.50 to 13.55.
- onAccent, minimum over accent and accentFill: light 4.52 (#E3008C) to 21.00, dark 4.52 (#E3008C) to 14.24.
- Style-fixed label (white light / black dark) on accentFill: light 4.50 (#FF8C00) to 21.00, dark 4.51 (#0067C0) to 21.00.

checks: fmt pass, clippy pass, tests 402 passed / 0 failed (6 skipped, nextest; includes the desktop `window_shell_loads_without_qml_errors` startup test), deny pass (advisories, bans, licenses and sources ok)
files changed: docs/design/design-system.md, apps/tinwave-desktop/qml/Theme/Theme.qml, apps/tinwave-desktop/qml/Main.qml, apps/tinwave-desktop/qml/Sidebar.qml
signals added/changed: none
ui: design ref docs/design/design-system.md §3-4 and canvas Library-Windows.dc.html (content `border-top-left-radius: 8px` with a 1px border, nav row `height: 34px`). Components: native ApplicationWindow, ItemDelegate, TextField, ToolButton, Menu, Label, plus token-coloured Rectangles. No Tw*. OS checked: Windows 11 26200 at 150 %, FluentWinUI3, Qt 6.12, with `--color-scheme=light|dark`. The OS theme and accent were not touched.
  Screenshots, cropped to the DWM frame bounds and saved in the session scratchpad (not committed, since `apps/.../docs/screenshots` is outside scope; the lead can copy them):
  - light: C:\Users\FSYY\AppData\Local\Temp\claude\D--dev-Tinwave\53bf9027-c019-4dd4-82f1-5a72c6705b0c\scratchpad\light.png
  - dark: C:\Users\FSYY\AppData\Local\Temp\claude\D--dev-Tinwave\53bf9027-c019-4dd4-82f1-5a72c6705b0c\scratchpad\dark.png
  - keyboard (Tab x3 to the list, then Down moves the selection to Albums): C:\Users\FSYY\AppData\Local\Temp\claude\D--dev-Tinwave\53bf9027-c019-4dd4-82f1-5a72c6705b0c\scratchpad\kb-light.png
  - The 34 px sidebar rows render cleanly and the rounded top-left content corner with its hairline is visible in both schemes. The footer hairline is in the `separator` colour.
  - The focus ring was not visible in the keyboard capture because the window was inactive under PrintWindow. Keyboard navigation itself worked, and the focus structure is unchanged from T-0040 apart from the row height.
  - Not done: a non-default-accent screenshot. There is no accent override switch yet, so accent behaviour is covered by the self-check numbers only. The other native styles (Fusion/macOS) were not re-checked; only row height and Rectangles changed.
docs updated: docs/design/design-system.md
deviations from ticket:
- The ticket says "update the prototype Theme.qml self-test". The spike prototype lies outside scope and is reference only, so the self-test now lives in the production `Theme.selfCheck()`, with the harness procedure documented in §3.1. This is the "documented check" the ticket asks for. The harness file itself is not committed (outside scope).
- onAccent is computed against both the raw `accent` and `accentFill`. The prototype computed it against `accent` only, but Main.qml feeds `palette.highlightedText: Theme.onAccent` over `palette.highlight: Theme.accentFill`. For a pale accent such as yellow in light that would give ink on dark gold, about 3.2:1. It is still one token.
- `selected` backs off its alpha only for extreme accents (the white accent in dark) so textFaint stays >= 4.5. Normal accents get exactly 10 %/12 %.
- The hover overlay counts as a surface for the contrast rule. That is what moved textFaint, srcDownload and warning, and it is the only reason `warning` changed.
- §5 and §7: the Tw focus ring and accent-coloured glyphs now use `accentText`, not raw `accent`, so they stay >= 3:1 with any accent. A pale yellow accent was 1.46:1 on light.
- Default window size is 1200 x 800 (previously a workaround, effectively 1200 x 880).

Look changes for the PO (token look changes are an escalation):
1. Light textFaint #6E6E77 -> #64646D. It is now very close to textMuted #5D5D66, so the tertiary/secondary step is weaker in light.
2. Light srcDownload #2E7D46 -> #2A7240.
3. Light warning #9A5B00 -> #925700.
4. Dark textFaint #8C8C95 -> #94949C.
5. `selected` is now documented as 10 % (light) and 12 % (dark) with hexes #E7F1F2 and #1E2B2E. Rendered values are unchanged from the prototype; the old hexes were within 1 level.
6. The content surface now has a rounded top-left corner (8 px on Windows) with a 1 px `separator` outline on its top and left edges, as in the canvas. The transport-bar line is now `separator` rather than the style's inset colour. Sidebar rows are 34 px instead of 40.

New tokens in Theme.qml and design-system §3.3: `windowDefaultWidth` 1200, `windowDefaultHeight` 800, `sidebarRowHeight` 34, `hairline` 1, `radiusContent` (= `radiusSurface`: 8 / 10 / 8). The `Main.qml:16` width-as-height workaround is removed.

proposals:
- P: Commit the contrast harness as a real test (qmltestcase or a `tests/` run of `Theme.selfCheck`) | why: the gate is documented but runs by hand; a CI test would stop token regressions | size: S | evidence: docs/design/design-system.md §3.1 "The check", Theme.qml selfCheck
- P: Decide whether light textFaint should keep a visible step below textMuted, e.g. darken textMuted to about #54545D | why: hover-aware contrast pushed textFaint to #64646D, almost equal to textMuted #5D5D66 | size: S | evidence: Theme.qml:resolve, hover/sidebar worst case 4.57 vs 5.09
- P: Screenshot capture helper that crops to DWMWA_EXTENDED_FRAME_BOUNDS and drives keys with the window active | why: reviewers also proposed it; my scratch PowerShell script cap.ps1 (in the scratchpad) works | size: S | evidence: scratchpad cap.ps1, cap2.ps1
- P: An accent-override dev switch (`--accent=#rrggbb`) alongside `--color-scheme` | why: lets future UI tickets screenshot pale/dark accents for the DoD "non-default OS accent" item | size: S | evidence: Main.qml Component.onCompleted

questions for lead:
- Items 1 to 4 under "Look changes for the PO" are small tint changes needed to meet 4.5:1, and the rounded content corner and 34 px rows follow the canvas. Do you want to escalate any of them to the PO, or are they pre-approved by ADR 0007 / D-0006 (which names the textFaint and srcDownload fixes)? The `warning` change and the hover-surface rule go beyond what the ADR named.
- Should ADR 0007's suggested hexes (textFaint #6A6A73, srcDownload #2B7842) be annotated as superseded by the hover-aware values (#64646D, #2A7240)? ADR 0007 is outside my scope.
- The T-0040 ticket text still says "Fusion elsewhere" (carried from the T-0040 round-2 review). It is not fixed here.

Cleanup: every tinwave-desktop process I started was killed, and none are left running. No Windows settings were changed, and no ignored or real-device tests were run. Nothing was pushed.

---
Lead (2026-11-02):
- Look changes 1-4: accepted, no escalation. D-0006/ADR 0007 already approved fixing the contrast failures; the warning
  tint and counting the hover overlay as a surface apply the same rule (text >= 4.5:1 wherever it can sit). Reported to
  the PO as FYI in the digest. Items 5-6 follow the approved canvas.
- ADR 0007's suggested hexes: the lead annotates them as superseded on merge.
- T-0040 ticket text: already fixed by the lead.
- Self-check in production Theme.selfCheck() with the documented harness: accepted as the "documented check".
- Proposals: P-0129 (harness as a real test) accepted -> T-0045; P-0130 (textFaint vs textMuted step) parked for the
  PO's look pass; P-0131 (capture helper) merged -> T-0043; P-0132 (--accent switch) merged -> T-0036.
- Screenshots: the lead copies the scratchpad light/dark/kb-light PNGs into apps/tinwave-desktop/docs/screenshots on merge.
