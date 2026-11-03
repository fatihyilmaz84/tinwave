HANDOFF T-0055 (round 2)
status: done
branch: t/T-0055-canvas-prose-and-render-harness   commit: 9fd6939
summary: A screenshot run can no longer reach the network: render() drops remote resources and prints a note for each;
the browser runs with `--host-resolver-rules="MAP * ~NOTFOUND"`; the Google Fonts `<link>` is removed from
Radio-Linux.dc.html; a stale PNG can no longer pass for a fresh one. Round-1 work unchanged.

round-2 items:
1. `dropRemote()` removes tags loading over http(s) (`<link href>`, `<img src>`, `<script src>`, `<iframe src>`…), CSS
   `@import` of http(s) or `//` URLs, and replaces `url(http(s)://...)` with `none`; each drop prints
   `<file>: dropped remote resource, <kind>: <snippet>` on stderr. Proof: Radio-Linux.dc.html with the link → note
   printed, output has 0 `googleapis`; a test file with six remote forms → all six dropped with notes; a plain
   `<a href="https://…">` kept.
2. `--host-resolver-rules=MAP * ~NOTFOUND` passed to the browser; all 7 screenshots render with it.
3. README documents dropped resources (local or fallback font), the resolver rule and the stale-PNG behaviour.
4. The `<link>` removed from Radio-Linux.dc.html (1 deletion; font list still falls back to Cantarell, system-ui,
   sans-serif); `grep -c http` on the rendered Radio-Linux.html is 0.
5. Stale PNG: the old `<Name>.png` is deleted before the browser spawns and a non-zero `r.status` fails the run (proof:
   `TINWAVE_BROWSER` = node.exe (exit 9) → "screenshot of … failed (exit 9)", exit 1, no PNG left).

checks: fmt pass, deny pass. Node v20.19.6, installed headless Edge; nothing installed.
files changed (round 2): docs/design/tools/render-canvas.js, docs/design/tools/README.md, docs/design/canvas/Radio-Linux.dc.html
screenshots: scratchpad\r2all\ (7 boards)
deviations: Radio-Linux.dc.html edited (scope widened by the lead). Two stray empty files from a bash quoting error were
created and deleted before the commit.
proposals: P-0226..P-0228 (restated).

---
Lead (2026-11-03): round-2 diff (44 lines) checked by the lead instead of a second review round: `child_process` is
used only to spawn the browser, no network APIs, the browser runs with the resolver block, and a fresh render of
Radio-Linux contains 0 remote URLs. Merged f155dce. Lead applied P-0226..P-0228 on main: Main board height 1900 → 2400
in canvas.json and `$preview` (the screen row and its title moved down 500 so nothing overlaps), Platforms "Qt 6.10+",
radius `content` row; all seven boards re-rendered with the harness (exit 0).
