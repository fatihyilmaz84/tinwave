HANDOFF T-0012 (round 1)
status: done
branch: t/T-0012-gitattributes   commit: 9d1a3f8
summary: Added `.gitattributes` with `* text=auto eol=lf`, explicit `text eol=lf` for rs/toml/md/qml/sql/json/html/svg, and `binary` for png/jpg/ico/flac/mp3/m4a/wav/ogg/db. The commit adds only that one file and changes no stored blobs, because the index was already all LF. Renormalize produced no changes, so merges with the T-0003, T-0004 and T-0013 branches should stay clean.

acceptance criteria:
- [x] `git add --renormalize . && git status --short` shows no change after the commit. Proof: it printed nothing after the commit. `git diff --stat HEAD~1 HEAD` shows only `.gitattributes | 24 ++++`. Before the commit, the same command showed only `A  .gitattributes`.
- [x] `git ls-files --eol` shows `i/lf` for every text file and `-text` for the binary patterns. Proof:
  - Index, before: 108 files all `i/lf` (working tree `w/crlf` from autocrlf).
  - Index, after: 109 files all `i/lf`. The extra one is `.gitattributes` itself, and `git ls-files -s` was not needed because no blob changed.
  - The repo has no tracked binary files, so `-text` cannot appear in `ls-files` yet. I checked the binary rules with `git check-attr -a` on dummy paths. `x.png`, `x.flac`, `x.db` and `x.ogg` show `binary: set`, `text: unset`, `diff: unset`, `merge: unset`. `x.rs`, `x.qml` and `x.svg` show `text: set`, `eol: lf`.
- [x] A fresh checkout followed by `cargo fmt --all --check` passes on Windows with autocrlf on.
  - I ran `git worktree add --detach` into `%TEMP%\tw-T0012-fresh` at 9d1a3f8, with global `core.autocrlf=true`.
  - All 109 files came out `i/lf w/lf`. Before the change they came out `w/crlf`.
  - `cargo fmt --all --check` exited 0.
  - I removed that worktree with `git worktree remove --force`. `git worktree list` shows only main and the four ticket worktrees, and I made no extra branches.

checks: fmt pass (run in the fresh checkout), clippy n/a (no code changed), tests 0 passed / 0 failed (no code changed), deny n/a
files changed: .gitattributes
signals added/changed: none
ui: none
docs updated: none
deviations from ticket: none. Renormalize would not change any stored content, so there is no file list to report.

proposals:
- P: One-time working-tree refresh after merging | why: the other worktrees (T-0003, T-0004, T-0013, main) still have CRLF files on disk. `.gitattributes` only affects them after a re-checkout, but their stored blobs are unaffected | size: S | evidence: `git ls-files --eol` showed `w/crlf` on all 108 files before the change. This matters little because `cargo fmt` already passed in them.

questions for lead:
- none
