REVIEW T-0035 (round 1)
verdict: CHANGES_REQUESTED
checks (run by reviewer): fmt pass (exit 0), clippy pass (`-D warnings`, exit 0), tests pass (nextest: 402 run, 402 passed, 6 skipped, including the desktop startup test), deny pass (advisories, bans, licenses and sources ok)

acceptance criteria:
- [x] The self-test passes for 15 accents × 2 schemes on every surface (≥ 4.5:1 text, ≥ 3:1 UI glyphs), for the surfaces it currently defines — verified by: my own harness (`qml -I <scratch> check.qml`, using a copy of the branch Theme.qml with the T-0011 SelfTest accent list) printed `SELFCHECK failures=0`, and all 30 rows match the handoff numbers. A random sweep of 3000 extra accents × 2 schemes through `selfCheck` gave 0 failures. HC key parity is true. I recomputed a sample of ratios independently in Python (WCAG luminance) and they agree:
  - light textFaint #64646D on hover/sidebar #E3E3E6 = 4.573 (old #6E6E77 on sidebar = 4.282)
  - light srcDownload #2A7240 = 4.581 (old 4.304)
  - light warning #925700 = 4.573 (old #9A5B00 = 4.236)
  - dark textFaint #94949C on hover/raised #2D2D30 = 4.560 (old 4.119)
  - the hover composites (#E3E3E6, #2D2D30) and the `selected` hexes #E7F1F2 / #1E2B2E reproduce from 10 % / 12 %
  - `onAccent` is computed against both `accent` and `accentFill` (Theme.qml:163). `selected` is backed off only when the nominal alpha fails (Theme.qml:107-116); in the run only the dark #FFFFFF, #F0E68C and #99FF99 rows sit at the floor. Alpha values are `Qt.rgba` in code and `#AARRGGBB` in the doc, and they match.
  - The surface set has one gap (blocking finding 1).
- [x] design-system.md §3 updated; changed look noted for the PO — verified by: diff of §3.1/§3.2/§3.3/§5/§7. They are consistent with Theme.qml (tokens, `selfCheck` description, new size and radius tokens). Look changes are listed in build-1.md and the lead accepted them.
- [x] P-0116 shell tokens — verified by: Theme.qml:282-285 and :291 (`windowDefaultWidth/Height`, `sidebarRowHeight`, `hairline`, `radiusContent`). They are used in Main.qml:15-16, :57-69, :88-93 and Sidebar.qml:100. The width-as-height workaround and the ToolSeparator are gone. Screenshots measure 34 logical px per row and a 1200 × 800 client area at 150 %.

blocking findings:
- docs/design/design-system.md:90-93 and apps/tinwave-desktop/qml/Theme/Theme.qml:92-102 — the contrast rule now counts the `hover` overlay as a surface, but only over the four fixed surfaces, not over `selected`.
  - Failure scenario: a hovered selected row is the most common row state, because the mouse is still on the row you just clicked. A future `TwTrackRow` that composites `hover` over `selected` (the doc does not say otherwise) puts text below 4.5:1, while the doc says every text token is ≥ 4.5:1 "on every surface text can sit on". Measured with the default accent:
    - dark Tin Teal: hover over selected = #2B373A, so textFaint is 4.08:1 (confirmed independently in Python)
    - dark #FFFFFF accent: 3.77
    - dark #99FF99: 3.77
    - dark #FFB900: 3.93
    - light #000000: 4.31
    - dark accentText with #10893E: 4.46
  - Suggested fix (S): either (a) state in §3.1 (and the §5 states line) that Tw components never composite `hover` over `selected` (the selected row shows `selected` only), and say so in the `surfacesOf` comment; or (b) add `hover/selected` to `surfacesOf` so `selfCheck` gates it. (b) will force token or `selected` changes.

design check: findings above.
- The token grep is clean: there are no hex colours, `Qt.rgba` or numeric sizes in qml/ outside Theme/. The only bare numbers are the pre-existing `spacing: 0` and `currentIndex: 0`.
- Native controls are not restyled. The only custom drawing is token-coloured shell Rectangles, the same kind T-0040 already accepted.
- The builder screenshots (light, dark, kb-light) match the canvas: 34 px rows, a rounded top-left content corner with a hairline, and the footer hairline in `separator`. They were taken with a non-default OS accent (purple).
- I also captured Fusion light and dark with `QT_QUICK_CONTROLS_STYLE=Fusion`. The 34 px rows render without clipping, and the highlighted row's label stays legible on `accentFill` in both schemes.
- Every process I started has exited.

non-blocking findings:
- docs/design/design-system.md:94-100 — the documented check does not give the hexes for the six Windows palette picks and the default blue, so two people can run "the" check with different inputs. List all 15 hexes (the T-0011 SelfTest list: #0067C0 #0B6E78 #FFB900 #E3008C #10893E #D13438 #FF8C00 #767676 #00B7C3 #FFFFFF #000000 #F0E68C #7A7574 #5CC6CF #99FF99), or expose them as a Theme constant. This also feeds T-0045.
- apps/tinwave-desktop/qml/Theme/Theme.qml:308 — the comment still says "2 px accent ring", but §5 now says the ring uses `accentText`.
- Theme.qml:282-283 — the first-start window of 1200 × 800 plus the title bar is taller than the available height on a 1366 × 768 screen at 100 %. It is still better than the old ~1200 × 880.

proposals:
- P: Sync the canvas token sheets with §3.1 | why: docs/design/canvas/Main.dc.html:199-208, Components.dc.html, Library-Windows.dc.html, Album-macOS.dc.html and others still use #6E6E77, #E6F1F2, #2E7D46, #9A5B00, #8C8C95, #1B2B2D and CSS-order alpha (`#0000000A`), so the design reference now contradicts the token table | size: S
- P: Clamp the first-start window size to `Screen.desktopAvailableWidth/Height` | why: the 1200 × 800 default overflows small laptop screens | size: S

notes for lead:
- Scope is clean: only the four files in scope changed, and nothing under `work/`.
- The fix for the blocking finding can be doc-only (option a); a round 2 should take minutes.
- Your planned ADR 0007 annotation should also cover `warning` and the `hover`-as-surface rule, which the ADR did not name.
- Harness and screenshots: scratchpad\rev35 (check.qml, wcag.py, fusion-light.png, fusion-dark.png, fluent-dark.png; out.txt).

---
Lead (2026-11-02): blocking finding → option (a): a selected row shows `selected` only, never `hover` composited
over it (native ItemDelegate keeps its own states). Round 2 also takes: the 15 hexes as a Theme constant used by
`selfCheck` and named in §3.1; the Theme.qml:308 comment; clamp the first-start window to the available screen
(P-0134). Canvas sync → P-0133 → T-0046.
