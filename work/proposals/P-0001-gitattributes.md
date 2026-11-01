---
id: P-0001
title: Add .gitattributes to pin line endings
from: T-0001 builder
date: 2026-11-01
size: S
outcome: accepted → T-0012
---

## Why
Pins line endings so fmt, clippy and diffs are stable across Windows `core.autocrlf`, macOS and Linux.

## Evidence
Git warns "LF will be replaced by CRLF" on every committed file (T-0001 branch, and the initial commit).

## Suggested approach
`* text=auto eol=lf`, `*.rs text eol=lf`, binary markers for images/audio.
