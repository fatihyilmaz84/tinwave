#!/usr/bin/env node
'use strict';
// Renders the design canvas (docs/design/canvas/*.dc.html) to plain, self-contained HTML, and optionally
// screenshots it with a locally installed headless Chromium (Edge or Chrome).
//
// The .dc.html files need the claude.ai canvas runtime (./support.js, not in the repo). This script replaces
// that runtime for the only two constructs the canvas uses: <sc-for list="{{path}}" as="x"> loops and {{path}}
// substitutions, with values taken from the file's own `renderVals()`.
//
// Dev-only tool: Node >= 18, no npm packages, no network. It reads and writes local files only.
// Usage: see README.md.

const fs = require('fs');
const os = require('os');
const path = require('path');
const { spawnSync } = require('child_process');
const { pathToFileURL } = require('url');

const get = (obj, dotted) => dotted.split('.').reduce((acc, key) => (acc == null ? acc : acc[key]), obj);

// Runs the file's <script type="text/x-dc"> and returns what renderVals() returns. The script is code from this
// repository (the same file the canvas runtime would run); it is evaluated here only with a stub DCLogic base class.
function loadValues(html, file) {
  const m = html.match(/<script type="text\/x-dc"[^>]*>([\s\S]*?)<\/script>/);
  if (!m) return {};
  const factory = new Function('DCLogic', `${m[1]}\nreturn new Component().renderVals();`);
  try {
    return factory(class DCLogic {});
  } catch (e) {
    throw new Error(`${file}: renderVals() failed: ${e.message}`);
  }
}

// Expands <sc-for> (nesting-aware) and {{path}}; throws on anything it cannot resolve, so a silently empty
// swatch or a half-rendered board can't be mistaken for a good render.
function expand(s, ctx, file) {
  for (;;) {
    const start = s.indexOf('<sc-for');
    if (start < 0) break;
    const tagEnd = s.indexOf('>', start);
    const tag = s.slice(start, tagEnd + 1);
    const list = /list="\{\{([^}]+)\}\}"/.exec(tag);
    const as = /\bas="([^"]+)"/.exec(tag);
    if (!list || !as) throw new Error(`${file}: unsupported <sc-for> tag: ${tag}`);
    let depth = 1;
    let end = -1;
    const re = /<sc-for|<\/sc-for>/g;
    re.lastIndex = tagEnd + 1;
    for (let m; (m = re.exec(s)); ) {
      if (m[0] === '<sc-for') depth++;
      else if (--depth === 0) {
        end = m.index;
        break;
      }
    }
    if (end < 0) throw new Error(`${file}: unclosed <sc-for>: ${tag}`);
    const items = get(ctx, list[1]);
    if (!Array.isArray(items)) throw new Error(`${file}: <sc-for list="${list[1]}"> is not an array in renderVals()`);
    const body = s.slice(tagEnd + 1, end);
    const rendered = items.map((it) => expand(body, Object.assign({}, ctx, { [as[1]]: it }), file)).join('');
    s = s.slice(0, start) + rendered + s.slice(end + '</sc-for>'.length);
  }
  return s.replace(/\{\{([A-Za-z0-9_.]+)\}\}/g, (_, p) => {
    const v = get(ctx, p);
    if (v == null) throw new Error(`${file}: {{${p}}} has no value in renderVals()`);
    return String(v);
  });
}

// Remote resources are never fetched: a render must work offline and must not call out. Drops
// - tags that load something over http(s) (<link href>, <img src>, <script src>, <iframe src>, ...);
// - CSS @import of an http(s) URL, and url(http(s)://...) values (replaced by `none`).
// Plain links (<a href>) and text are left alone. Returns the cleaned HTML and a description of each drop.
const REMOTE = '(?:https?:)?//';
const REMOTE_TAG = new RegExp(
  '<(?:link|img|script|iframe|source|video|audio|embed|object|image|use)\\b[^>]*?\\b(?:href|src|data|xlink:href)\\s*=\\s*["\']?' +
    REMOTE +
    '[^>]*>',
  'gi',
);
const REMOTE_IMPORT = new RegExp('@import\\s+(?:url\\(\\s*)?["\']?' + REMOTE + '[^;]*;?', 'gi');
const REMOTE_URL = new RegExp('url\\(\\s*["\']?' + REMOTE + '[^)]*\\)', 'gi');

function dropRemote(html) {
  const dropped = [];
  const drop = (kind, replacement) => (m) => {
    dropped.push(`${kind}: ${m.replace(/\s+/g, ' ').slice(0, 120)}`);
    return replacement;
  };
  html = html.replace(REMOTE_TAG, drop('tag', ''));
  html = html.replace(REMOTE_IMPORT, drop('@import', ''));
  html = html.replace(REMOTE_URL, drop('url()', 'none'));
  return { html, dropped };
}

function render(file) {
  let html = fs.readFileSync(file, 'utf8');
  const vals = loadValues(html, file);
  html = html
    .replace(/<script type="text\/x-dc"[\s\S]*?<\/script>/, '')
    .replace(/<script src="\.\/support\.js"><\/script>/, '');
  html = expand(html, vals, file);
  const cleaned = dropRemote(html);
  html = cleaned.html;
  for (const d of cleaned.dropped) console.error(`${path.basename(file)}: dropped remote resource, ${d}`);
  const leftover = /<sc-[a-z-]+|\{\{/.exec(html);
  if (leftover) throw new Error(`${file}: unsupported canvas construct left in output: ${leftover[0]}`);
  return html;
}

function findBrowser() {
  const candidates = [
    process.env.TINWAVE_BROWSER,
    'C:/Program Files (x86)/Microsoft/Edge/Application/msedge.exe',
    'C:/Program Files/Microsoft/Edge/Application/msedge.exe',
    'C:/Program Files/Google/Chrome/Application/chrome.exe',
    '/Applications/Microsoft Edge.app/Contents/MacOS/Microsoft Edge',
    '/Applications/Google Chrome.app/Contents/MacOS/Google Chrome',
    '/usr/bin/microsoft-edge',
    '/usr/bin/google-chrome',
    '/usr/bin/chromium',
    '/usr/bin/chromium-browser',
  ];
  return candidates.find((c) => c && fs.existsSync(c));
}

function screenshot(browser, htmlFile, pngFile, width, height) {
  // A throw-away profile so the run neither touches nor waits on the user's own browser profile.
  const profile = fs.mkdtempSync(path.join(os.tmpdir(), 'tinwave-canvas-'));
  // A stale PNG from an earlier run must never make a failed run look successful.
  fs.rmSync(pngFile, { force: true });
  try {
    const r = spawnSync(
      browser,
      [
        '--headless=new',
        '--disable-gpu',
        '--hide-scrollbars',
        '--no-first-run',
        '--disable-sync',
        '--disable-background-networking',
        '--disable-component-update',
        // Belt and braces: even if a remote reference slipped through, no host name resolves.
        '--host-resolver-rules=MAP * ~NOTFOUND',
        `--user-data-dir=${profile}`,
        `--window-size=${width},${height}`,
        `--screenshot=${pngFile}`,
        pathToFileURL(htmlFile).href,
      ],
      { stdio: 'ignore', timeout: 60000 },
    );
    if (r.error || r.status !== 0 || !fs.existsSync(pngFile)) {
      throw new Error(`screenshot of ${htmlFile} failed (${r.error ? r.error.message : 'exit ' + r.status})`);
    }
  } finally {
    fs.rmSync(profile, { recursive: true, force: true });
  }
}

function main() {
  const args = process.argv.slice(2);
  const shoot = args.includes('--screenshot');
  const heightArg = args.find((a) => a.startsWith('--height='));
  const heightOverride = heightArg ? Number(heightArg.slice(9)) : 0;
  if (heightArg && !(heightOverride > 0)) throw new Error(`bad ${heightArg}`);
  const [input, outDir] = args.filter((a) => !a.startsWith('--'));
  if (!input || !outDir || args.includes('--help')) {
    console.error('usage: node render-canvas.js <file.dc.html | canvas-dir> <out-dir> [--screenshot] [--height=N]');
    process.exit(2);
  }
  const isDir = fs.statSync(input).isDirectory();
  const dir = isDir ? input : path.dirname(input);
  let boards = {};
  const manifest = path.join(dir, 'canvas.json');
  if (fs.existsSync(manifest)) boards = JSON.parse(fs.readFileSync(manifest, 'utf8')).boards || {};
  const files = isDir
    ? Object.keys(boards).length
      ? Object.keys(boards)
      : fs.readdirSync(input).filter((f) => f.endsWith('.dc.html'))
    : [path.basename(input)];

  const browser = shoot ? findBrowser() : null;
  if (shoot && !browser) {
    console.error('no Edge/Chrome/Chromium found; set TINWAVE_BROWSER to its executable path');
    process.exit(1);
  }
  fs.mkdirSync(outDir, { recursive: true });
  for (const f of files) {
    const name = f.replace(/\.dc\.html$/, '');
    const htmlOut = path.resolve(outDir, `${name}.html`);
    fs.writeFileSync(htmlOut, render(path.join(dir, f)));
    console.log(htmlOut);
    if (shoot) {
      const size = boards[f] || { w: 1440, h: 900 };
      const png = path.resolve(outDir, `${name}.png`);
      screenshot(browser, htmlOut, png, size.w, heightOverride || size.h);
      console.log(png);
    }
  }
}

try {
  main();
} catch (e) {
  console.error(e.message);
  process.exit(1);
}
