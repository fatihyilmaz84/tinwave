---
id: D-0009
title: Tinwave's own licence — GPL-3.0 (GitHub LICENSE) or MIT OR Apache-2.0 (Cargo.toml, packaging)?
status: open
blocks: [first public release; D-0007]
date: 2026-11-03
---

## Question for the PO
The GitHub repository (https://github.com/fatihyilmaz84/tinwave) was created with a GPL-3.0 `LICENSE` (initial commit
e0b8e4b, now merged into main). The project itself declares `license = "MIT OR Apache-2.0"` in the root Cargo.toml, every
crate inherits it, and the Windows package ships an Apache-2.0 text as Tinwave's own licence (T-0042/T-0051). Which one
is Tinwave's licence?

## What each choice means
- **GPL-3.0:** compatible with everything we use (Qt LGPL-3 dynamic, MPL-2.0 Symphonia, MIT/Apache crates, MIT
  Signalsmith). Anyone who ships a modified Tinwave must publish their source under GPL-3.0. It would also allow GPL-only
  components later (e.g. Rubber Band, which ADR 0006 avoided for licence reasons). Change Cargo.toml to `GPL-3.0-only`
  (or `-or-later`), add the notice header guidance, update packaging (ship GPL-3.0 as Tinwave's licence) and deny.toml's
  view of our own crates.
- **MIT OR Apache-2.0:** what the code and docs assume today; most permissive. Replace the GitHub LICENSE with
  LICENSE-MIT + LICENSE-APACHE (D-0007 then only needs the holder name and contact).

## Lead recommendation
No strong technical preference — both work with every dependency. The difference is about what others may do with
Tinwave. If you want modified versions to stay open, GPL-3.0; if you want maximum reuse, MIT OR Apache-2.0. Whichever
you pick, the lead aligns Cargo.toml, packaging and docs in one change.

## PO decision
_pending_
