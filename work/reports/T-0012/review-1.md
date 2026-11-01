REVIEW T-0012 (round 1)
verdict: APPROVE
checks (run by reviewer): fmt pass (exit 0 in a fresh detached checkout of 9d1a3f8 with global core.autocrlf=true), clippy pass (`-D warnings`, exit 0, same checkout), tests pass (21 passed / 0 failed, exit 0), deny pass ("advisories ok, bans ok, licenses ok, sources ok")

acceptance criteria:
- [x] `git add --renormalize . && git status --short` shows no change after the commit. Verified by running it myself in the fresh checkout at 9d1a3f8: it printed nothing and exited 0. `git diff main...t/T-0012-gitattributes` changes one file, `.gitattributes` (+24). `git grep -I -l $'\r'` found no CR in any blob on main, T-0003, T-0004, T-0008 or T-0013. I also checked the T-0003 `.sh` and `.txt` blobs with `cat-file`: 0 CRs. So no stored content changes.
- [x] `git ls-files --eol` shows `i/lf` for every text file, and the binary patterns resolve to `-text`. Verified in the builder worktree: 109/109 `i/lf`, 104 `attr/text`, 5 `attr/text=auto`. No binary files are tracked, so `-text` cannot show up in `ls-files` yet. I checked with `git check-attr` instead:
  - `x.png/jpg/ico/flac/mp3/m4a/wav/ogg/db` give `binary: set, text: unset, diff: unset, merge: unset`.
  - `x.rs/toml/md/qml/sql/json/html/svg` give `text: set, eol: lf`.
  - The binaries also show `eol: lf`, inherited from `*`. That is harmless because `text: unset` turns conversion off.
- [x] A fresh checkout followed by `cargo fmt --all --check` passes on Windows with autocrlf on. Verified with `git worktree add --detach` at 9d1a3f8 into my scratchpad under %TEMP%:
  - `core.autocrlf` was `true`, and all 109 files came out `i/lf w/lf`.
  - fmt, clippy, tests and deny all exited 0.
  - I then ran `git worktree remove`, which exited 0, and deleted its separate CARGO_TARGET_DIR. `git worktree list` no longer shows it.

blocking findings:
- none

design check: n/a

non-blocking findings:
- `.gitattributes` (whole file): there is no `*.bat` / `*.cmd` rule, so these files would be checked out LF through `* text=auto eol=lf`. cmd.exe with LF-only batch files misparses labels, and `GOTO`/`CALL :label` can fail when a label falls on a 512-byte read boundary. This is not blocking now: none of the six branches tracks any `.bat`/`.cmd`, and no ticket or guide plans one. The ticket's scope also lists the exact patterns, and the builder matched them. It should still be fixed before the first Windows script lands, for example an MSVC/Qt env wrapper for Phase 0 build commands: `*.bat text eol=crlf` and `*.cmd text eol=crlf`.
- `.gitattributes` (binary section): these extensions are not marked binary: `*.jpeg`, `*.gif`, `*.webp`, `*.ttf`, `*.otf`, `*.aac`, `*.opus`, `*.sqlite`, `*.db-wal`, `*.db-shm`. They currently resolve to `text=auto`. Real files of these types contain NUL bytes in the first 8000 bytes, so git's auto-detection treats them as binary and does not convert them. The risk is low, but explicit `binary` is more robust and documents intent. (`.alac` is not a real container extension, since ALAC lives in `.m4a`/`.caf`. SQLite artifacts should be gitignored anyway.)
- No problem for the other types you listed: `*.qrc`, `*.ui`, Qt `*.ts` (all XML), insta `*.snap`, `*.svg` and `*.ps1` all resolve to LF text, which is correct. PowerShell 5.1 runs LF scripts fine. For `.ps1`, the BOM/encoding matters, not the line endings.

proposals:
- P: Extend .gitattributes with CRLF for `*.bat`/`*.cmd` and explicit binary for jpeg/gif/webp/ttf/otf/aac/opus/sqlite/db-wal/db-shm | why: keeps cmd.exe scripts working and stops relying on content sniffing for binaries | size: S
- P: (builder's) one-time re-checkout of existing worktrees after merge | why: T-0003/T-0004/main still have `w/crlf` files on disk; this is cosmetic, since the stored blobs are LF and fmt passes | size: S

notes for lead:
- `git merge-tree --write-tree` against current main (6dc383b), T-0003 and T-0004 merges cleanly (exit 0), and main has no `.gitattributes`. It is safe to merge in any order.
- I made no edits or commits. I also ran no index-touching commands in the builder worktree: renormalize ran only in my temporary checkout. That checkout is removed, and the builder worktree is still clean at 9d1a3f8.
- The worktree list changed while I was reviewing (main moved to 6dc383b, and the T-0013 worktree is gone). That was not me.
