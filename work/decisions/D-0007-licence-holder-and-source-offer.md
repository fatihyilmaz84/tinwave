---
id: D-0007
title: Tinwave's copyright holder and the contact for the Qt written source offer
status: open
blocks: [first public release (not current work)]
date: 2026-11-03
---

## Questions for the PO (from T-0042)
1. Who is the copyright holder of Tinwave? Since D-0009 / ADR 0009 Tinwave is GPL-3.0-only: the GPL text names no
   holder, but the copyright notice in the package, the About view and the README must ("Copyright (C) <year>
   <holder>"). Because the closed mobile apps rely on the holder owning all the code, this is the person or company
   that will own Tinwave.
2. What contact (email or URL) goes into the Qt written source offer? LGPL-3 lets us point at Qt's own download site
   for the exact sources (done: download.qt.io 6.12.0 submodules), but a written offer also needs a way to reach us.

## Context
work/reports/T-0042/build-1.md; apps/tinwave-desktop/packaging/templates/SOURCE-OFFER-Qt.txt; ADR 0008 lead decision 2
(dynamic Qt, LGPL texts, THIRD-PARTY-NOTICES, source offer).

## Lead recommendation
1. Holder: your name, or your company if Tinwave will be sold through one. Not "The Tinwave contributors": the
   closed mobile apps need a single owner (ADR 0009).
2. Contact: a project URL once one exists; until then an email address you're happy to publish. Nothing ships before
   this is set, so it doesn't block current work.

## PO decision
_pending_
