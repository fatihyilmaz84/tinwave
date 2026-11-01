---
id: T-0020
title: "Spike: real-world format and decoder corpus (iTunes/ffmpeg M4A, libFLAC, VBR MP3, HE-AAC, Opus, AIFF, WavPack; Media Foundation AAC)"
type: spike
epic: E-01
status: blocked
priority: P1
size: M
depends_on: [T-0003]
scope:
  - spikes/T-0020-format-corpus/
owner:
branch:
review_round: 0
created_by: from P-0026
---

## Context
T-0003 had no ffmpeg and no real encoders: iTunes-style M4A head priming, libFLAC STREAMINFO, VBR MP3 seek, HE-AAC,
Opus/OGG/AIFF/WavPack are untested. D-0004 (AAC route) needs Media Foundation measured as the AAC/HE-AAC decoder.

## Question
Which real-world files does the T-0003 stack mishandle, and does Media Foundation work as the Windows AAC/HE-AAC route?

## Time box
1 working session.

## Acceptance criteria
- [ ] Format x (decode / seek / gapless) matrix on real files, with the source of each file noted (no copyrighted
      audio committed — only hashes, sizes and encoder strings).
- [ ] Media Foundation AAC/HE-AAC decode measured (gapless with trim, seek, CPU), license notes.
- [ ] Draft ADR addendum.

## Notes
Blocked on the PO: ffmpeg installed (or permission for the agent to install it) and a folder with a few real iTunes
M4A albums and other formats.

## Signals & storage
None.
