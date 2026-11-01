---
id: T-0012
title: Add .gitattributes that pins LF line endings and marks binaries
type: chore
epic: E-00
status: ready
priority: P1
size: S
depends_on: [T-0001]
scope:
  - .gitattributes
owner:
branch:
review_round: 0
created_by: from P-0001
---

## Context
The dev machine has `core.autocrlf=true`; macOS/Linux CI and agents will not (D-0001). Without pinned line
endings, `cargo fmt --check`, diffs and snapshot tests can differ per OS. docs/rules/testing.md, docs/rules/rust.md.

## Scope
- `.gitattributes`: `* text=auto eol=lf`; explicit `text eol=lf` for `*.rs`, `*.toml`, `*.md`, `*.qml`, `*.sql`,
  `*.json`, `*.html`, `*.svg`; `binary` for `*.png`, `*.jpg`, `*.ico`, `*.flac`, `*.mp3`, `*.m4a`, `*.wav`,
  `*.ogg`, `*.db`.
- Renormalize the index in the same commit if anything changes.

## Out of scope
Changing `rustfmt.toml` `newline_style` (proposal if needed). Any other file.

## Acceptance criteria
- [ ] `git add --renormalize . && git status --short` shows no change after the commit — verified by command output.
- [ ] `git ls-files --eol` shows `i/lf` for every text file and `-text` for the binary patterns — verified by command output.
- [ ] A fresh checkout (`git worktree add`) followed by `cargo fmt --all --check` passes on Windows with autocrlf on.

## Signals & storage
None.

## Notes for the builder
Spike branches are in flight; this file only affects checkout, not stored blobs (already LF), so it must not
cause merge conflicts. Confirm that with `git ls-files --eol` before and after.
