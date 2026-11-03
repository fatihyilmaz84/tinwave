---
id: D-0009
title: Tinwave's own licence — open-source desktop, closed-source mobile
status: open
blocks: [first public release; D-0007]
date: 2026-11-03
---

## Question for the PO
The GitHub repository (https://github.com/fatihyilmaz84/tinwave) was created with a GPL-3.0 `LICENSE` (initial commit
e0b8e4b, now merged into main). The code declares `license = "MIT OR Apache-2.0"` in the root Cargo.toml, and the
Windows package ships an Apache-2.0 text as Tinwave's licence (T-0042/T-0051). Which licence is Tinwave's?

## PO input (2026-11-03)
- The desktop app can be open source. The mobile apps will be closed source.
- The desktop app uses the free (LGPL-3) Qt.

## What that implies
- **Qt affects the desktop only.** Mobile UI is SwiftUI / Jetpack Compose (spec §2), so Qt's licence never reaches the
  mobile apps. Desktop obligations under free Qt stay as planned: dynamic linking, ship the Qt licence texts, a Qt
  source offer (D-0007 contact), and let users replace the Qt DLLs. Any open-source licence for the desktop works with that.
- **The shared Rust crates are the real question.** Closed mobile apps will reuse the engine crates (core, signals, db,
  playback, later taste/radio/sync) through FFI, and Rust links them statically.
  - As the only copyright holder, the PO can use their own code in a closed app whatever its public licence; the GPL binds
    others, not the owner. That stays true only while every line in those crates is the PO's: outside contributions
    would need a contributor agreement (CLA) before they could ship in the closed mobile apps.
  - **Third-party dependencies of the shared crates must stay closed-source-safe:** permissive (MIT/Apache/BSD/Zlib) or
    file-level MPL-2.0 (Symphonia) only; no GPL, and no LGPL (it does not fit static linking into an iOS app). The
    current `cargo deny` allow-list already enforces this; keep it. Rubber Band (GPL) stays out of `tinwave-playback`.

## Options
- **A. GPL-3.0 for the whole repository** (matches the existing GitHub LICENSE). Nobody else can turn Tinwave into a
  closed product; the PO can still reuse the crates in the closed mobile apps, but needs a CLA once outside
  contributors arrive.
- **B. MPL-2.0 for the shared crates, GPL-3.0 for `apps/tinwave-desktop`.** No CLA needed for mobile reuse; changes to
  the shared files must be published (also from the mobile apps), but the closed mobile code around them stays closed.
- **C. MIT OR Apache-2.0** (what the code says today). Simplest, but anyone may ship a closed fork of the engine.

## Lead recommendation
**A.** No outside contributors yet, so no CLA is needed today, and it keeps the desktop and engine from being closed by
others. Revisit B if contributors appear and a CLA is unwelcome. Mobile code goes in a separate private repository.
Applying it: Cargo.toml `license = "GPL-3.0-only"` (or `-or-later`), a short notice in the README, packaging ships
GPL-3.0 as Tinwave's licence next to the third-party notices, and `deny.toml` keeps its strict allow-list for
dependencies.

## PO decision
_pending_
