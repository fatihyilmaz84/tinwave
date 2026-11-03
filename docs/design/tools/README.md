# Design tools

## render-canvas.js — see the design canvas without the canvas runtime

`docs/design/canvas/*.dc.html` are design-canvas boards. They need the claude.ai canvas runtime (`./support.js`, not in
the repo), so opened on their own they show nothing useful. `render-canvas.js` stands in for that runtime and writes
plain HTML you can open in any browser, or screenshot headlessly.

**Prerequisite (dev-only):** Node.js 18 or newer. Nothing else: no npm packages, no network. Tinwave itself does not
need Node, and no build step touches this folder. The script reads the `.dc.html` files and writes to the output
folder you give it; it makes no network requests. (Tested with Node 20.19.)

### Render

```
node docs/design/tools/render-canvas.js docs/design/canvas <out-dir>                 # all boards, in canvas.json order
node docs/design/tools/render-canvas.js docs/design/canvas/Main.dc.html <out-dir>    # one board
```

Each board becomes `<out-dir>/<Name>.html` (for example `Main.html`, `Album-macOS.html`). Open it in a browser.

What it does: runs the board's own `<script type="text/x-dc">` (`renderVals()`), expands `<sc-for list="{{path}}"
as="x">` loops and `{{path}}` substitutions, and drops the runtime `<script>` tags. Those are the only two canvas
constructs the boards use. If a board uses anything else, a `{{path}}` with no value, or a `list` that is not an array,
the script stops with an error naming the file rather than writing a half-rendered page.

### Screenshot

Add `--screenshot` and the script also saves `<out-dir>/<Name>.png` with a locally installed headless Edge, Chrome or
Chromium (found in the usual install paths; set `TINWAVE_BROWSER` to an executable path to override). The window is the
board's size from `canvas.json` (1440 × 1900 for Main, 1440 × 900 for the screen boards), with a throw-away browser
profile and sync, background networking and component update switched off.

```
node docs/design/tools/render-canvas.js docs/design/canvas <out-dir> --screenshot
node docs/design/tools/render-canvas.js docs/design/canvas/Main.dc.html <out-dir> --screenshot --height=2500
```

`--height=N` overrides the window height (useful when a board's content is taller than its `canvas.json` size, which
the canvas would crop).

By hand, the equivalent for one rendered file is:

```
msedge --headless=new --disable-gpu --hide-scrollbars --window-size=1440,900 --screenshot=out.png file:///path/to/out/Album-macOS.html
```

Screenshots are evidence for a review, not part of the repo: keep them outside the tree.

### Limits

- This is a faithful render of the boards' HTML and CSS, not of the canvas app itself (no pan, zoom or placement; board
  positions in `canvas.json` are ignored).
- The system fonts named in each board's `font-family` are used if installed; otherwise the browser's fallback is, so
  glyph widths can differ slightly between machines.
