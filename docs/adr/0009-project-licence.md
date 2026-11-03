# ADR 0009 — Tinwave's own licence: GPL-3.0, closed-source mobile apps

- **Status:** Accepted
- **Date:** 2026-11-03

## Context

The GitHub repository was created with a GPL-3.0 `LICENSE`, while the workspace declared `MIT OR Apache-2.0` and the
Windows package shipped Apache-2.0 as Tinwave's licence (D-0009). The PO wants the desktop app open source and the
mobile apps closed source; the desktop uses the free (LGPL-3) Qt.

## Decision

1. **Tinwave's own code in this repository is `GPL-3.0-only`** (PO, D-0009, 2026-11-03). The root `LICENSE` holds the
   GPL-3.0 text; every crate inherits `license` from the workspace. "-only" keeps future licence versions a decision of
   the copyright holder rather than automatic.
2. **Mobile apps are closed source and live in a separate private repository.** They may reuse this repository's crates
   because the PO holds the copyright; the GPL binds licensees, not the owner. This holds only while the PO owns every
   line of those crates: **no outside contribution is merged without a contributor licence agreement (CLA)** that lets
   the PO ship it in closed apps.
3. **Dependencies stay closed-source-safe in every crate.** `deny.toml` keeps its allow-list (permissive licences plus
   MPL-2.0 for symphonia* only, D-0003). No GPL dependency — it would bind the PO too — and no LGPL in Rust crates,
   which link statically (LGPL relinking does not fit an iOS app). Rubber Band (GPL) stays out of `tinwave-playback`.
4. **Qt stays desktop-only and LGPL-3.** Mobile UI is SwiftUI / Jetpack Compose (spec §2). The desktop keeps ADR 0008's
   obligations (dynamic Qt, licence texts, Qt source offer). GPL-only Qt modules (e.g. Qt Charts, Qt Graphs) are not
   used, so the desktop app never depends on more than LGPL Qt.
5. `cargo deny` ignores our own unpublished crates (`[licenses.private] ignore = true`); every dependency is still
   checked against the allow-list.

## Consequences

- A binary release must carry, besides the third-party notices: the GPL-3.0 text as Tinwave's licence, a copyright
  notice naming the holder (D-0007), the way to get the exact corresponding source (the public repository at the
  release tag), and "Appropriate Legal Notices" in the UI (GPL-3.0 §0/§5d: an About view with the copyright, no
  warranty and the licence). Tracked in T-0054.
- Accepting contributions needs a CLA process first (not needed while the agent team is the only author).
- The packaging scripts, the committed `THIRD-PARTY-NOTICES.txt` and the D-0007 check switch from `LICENSE-MIT` /
  `LICENSE-APACHE` to the root `LICENSE` (T-0058).

## Spec / docs updated

- `tinwave-specs.md` §15: licence bullet.
- `docs/rules/architecture.md`: dependency licence rule.
- `work/decisions/D-0009` (decided), `D-0007` (holder now for the GPL copyright notice).
- Applied in code by T-0058: root `Cargo.toml` `license`, `deny.toml` `[licenses.private]`, packaging.
