---
id: D-0007
title: Tinwave's copyright holder and the contact for the Qt written source offer
status: open
blocks: [first public release (not current work)]
date: 2026-11-03
---

## Questions for the PO (from T-0042)
1. Who is the copyright holder named in Tinwave's own licence texts? The workspace declares "MIT OR Apache-2.0", but
   the repo root has no LICENSE-MIT / LICENSE-APACHE, and the MIT text needs a named holder ("Copyright (c) <year>
   <holder>"). The package currently ships only the Apache-2.0 text.
2. What contact (email or URL) goes into the Qt written source offer? LGPL-3 lets us point at Qt's own download site
   for the exact sources (done: download.qt.io 6.12.0 submodules), but a written offer also needs a way to reach us.

## Context
work/reports/T-0042/build-1.md; apps/tinwave-desktop/packaging/templates/SOURCE-OFFER-Qt.txt; ADR 0008 lead decision 2
(dynamic Qt, LGPL texts, THIRD-PARTY-NOTICES, source offer).

## Lead recommendation
1. Holder: your name (or "The Tinwave contributors" if you plan to accept outside contributions without a CLA);
   add LICENSE-MIT and LICENSE-APACHE at the repo root and ship both in the package.
2. Contact: a project URL once one exists; until then an email address you're happy to publish. Nothing ships before
   this is set, so it doesn't block current work.

## PO decision
_pending_
