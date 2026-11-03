HANDOFF T-0050 (round 1)
status: done
branch: t/T-0050-nav-focus-ring   commit: 2064da8
summary: The keyboard ring now follows the current row in the sidebar nav after Up/Down/PageUp/PageDown/Home/End, in light and dark. The list is still one tab stop, with native ItemDelegate visuals and accessible names; a mouse click shows no ring. Root cause: ListView already moves active focus to the new current row, but with a non-keyboard focus reason (7, OtherFocusReason); the native delegate draws its ring only for `visualFocus` (Tab/Backtab/Shortcut). `tools/capture.ps1` now verifies the foreground (retry, then exit 1 with a message).

acceptance criteria:
- [x] Keyboard walkthrough screenshots (light + dark, 150 %, tools/capture.ps1): Tab into the list, Down x3, Up x1, ring on the current row in every step — two montages, 5 steps each (Songs, Albums, Artists, Genres, Artists), checked by eye.
- [x] Mouse click shows no ring — scratch captures with a synthetic click: (a) Tab in (ring on Songs), click Genres → selected, no ring; (b) click Artists with no Tab → selected, no ring; (c) click then Down → ring on the next row (as natively). Not committed (scratch-only click helper).
- [x] Token grep clean; startup tests pass; fmt, clippy, tests, deny pass — grep for `#hex`, `pixelSize`, `rgba(`, `radius`, `duration`, `width|height|spacing|margins|padding: <n>` over Sidebar.qml: no hits.
- [x] P-0163: capture.ps1 checks the foreground before each keyboard capture; 5 retries with a short pause, then fails loudly. Scratch copy with the comparison forced to fail: 5 retry warnings, "capture aborted: Tinwave could not be brought to the foreground after 5 attempts ... Nothing was saved", exit 1, no PNG, no process left. In normal runs retries kicked in (warnings) and captures succeeded.

checks: fmt pass, clippy pass, tests 495 passed / 0 failed (6 skipped; cancel_storm passed this run), deny pass
files changed: apps/tinwave-desktop/qml/Sidebar.qml; apps/tinwave-desktop/tools/capture.ps1; apps/tinwave-desktop/docs/screenshots/keyboard-walkthrough-windows11-fluentwinui3-light.png (replaced; the old one showed the bug); …-dark.png (new)
signals added/changed: none (navigation is not a preference signal yet, per the ticket)

ui: design-system §5 and §7, canvas "Library · Windows" sidebar. Native ItemDelegate/ListView only, no Tw*, no tokens added, no raw values; the native FluentWinUI3 ring (black light, white dark) kept. Windows 11 26200, FluentWinUI3, 150 % (1800x1245), `--color-scheme=light|dark`; OS theme/accent untouched. Fusion and non-default accents not checked. Tab order unchanged (menu button, Search, list's current row); full keyboard-only walkthrough beyond that sequence and the click cases not redone. Deviations from the design: none.

How it works (Sidebar.qml):
- `keyboardMove` is true while a navigation key is down (`Keys.onPressed`/`onReleased` on the ListView).
- In `onCurrentIndexChanged`, if `keyboardMove` is set and the new current row lacks `visualFocus`, the row is re-focused with `Qt.TabFocusReason` (`focus = false`, then `forceActiveFocus(Qt.TabFocusReason)`). A mouse click never sets `keyboardMove`.
- Delegate `focusPolicy` is `ListView.isCurrentItem || activeFocus ? StrongFocus : ClickFocus`. The old binding logged "QQuickItem: Cannot set activeFocusOnTab to false once item is the active focus item." on every Down; gone, still one tab stop.

docs updated: capture.ps1 header comment.
deviations from ticket: the two PNGs live in apps/tinwave-desktop/docs/screenshots (outside the literal scope; evidence files where the old walkthrough lived).

proposals:
- P: capture.ps1 key-delivery check (`-Click x;y` step, post-key readiness probe) | why: the first key after activation is occasionally still lost | size: S | evidence: two bad dark captures, 3/3 good on rerun
- P: Reusable focus-follows-selection helper for later list screens | why: TwTrackRow lists will hit the same reason-7 issue | size: S | evidence: Sidebar.qml `onCurrentIndexChanged`

questions for lead: none. All committed screenshots re-taken with the final script and checked by eye; no processes left.

---
Lead (2026-11-03): looked at the dark montage — ring on the current row in all 5 steps. Screenshots in docs/screenshots
accepted. P-0175 (capture key-delivery check) parked → next UI ticket that needs keyboard evidence; P-0176 (focus-
follows-selection helper) parked → input to the first list-screen ticket (TwTrackRow).
