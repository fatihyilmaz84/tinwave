REVIEW T-0058 (round 1)
verdict: APPROVE
checks (run by reviewer): fmt pass (exit 0), clippy -D warnings pass (exit 0), tests pass (nextest: 622 run, 622 passed, 6 skipped), deny pass ("advisories ok, bans ok, licenses ok, sources ok", exit 0). Packaging: a full `package.ps1` (release build + smoke test) exited 0, and `test-packaging.ps1` ran 37 cases with 0 failed.

acceptance criteria:
- [x] `cargo metadata` reports GPL-3.0-only for every workspace crate, and `cargo deny check` passes with the allow-list unchanged — verified by: `cargo metadata --no-deps` lists tinwave-context, -core, -signals, -db, -playback, -test-alloc and -desktop, all `GPL-3.0-only` with `publish=[]`. `git diff main -- deny.toml` shows only additions: the comment and `[licenses.private] ignore = true` at D:\dev\Tinwave-worktrees\T-0058-switch-to-gpl\deny.toml:29-32. `allow` (lines 16-26) and the symphonia MPL exceptions are untouched.
- [x] Negative check: a GPL dependency still fails `cargo deny` — verified by: in a scratch git-archive copy I added a path crate `gpl-pub` (GPL-3.0-only, publishable) as a dependency of tinwave-core. `cargo deny check licenses` gave "rejected: license is not explicitly allowed", exit 4. Then I deleted the copy.
- [x] The package has the GPL-3.0 text as Tinwave's licence and no MIT/Apache text presented as Tinwave's; D-0007 is still PENDING on holder and contact only — verified by:
  - `licenses\` in the fresh bundle holds exactly `Qt-LGPL-3.0-and-GPL-3.0.txt` and `Tinwave-LICENSE-GPL-3.0.txt`. The latter has the same SHA-256 as the root `LICENSE` ("GNU GENERAL PUBLIC LICENSE Version 3, 29 June 2007"). `Apache-2.0.txt` is gone.
  - check-bundle printed "PENDING d0007 ... copyright holder of Tinwave's own code ...; contact for the Qt written source offer ..." with no LICENSE-MIT or LICENSE-APACHE.
  - release-identity.json is unchanged (holder and contact still null). Nothing was invented.
  - Qt/ADR 0008 obligations are untouched: the Qt licence copy and source-offer code are not in the diff, and PASS legal, source-offer, qt-unmodified, gpl-only and static-qt all hold.
- [x] `package.ps1` and `test-packaging.ps1` exit 0, and the committed `THIRD-PARTY-NOTICES.txt` matches a fresh generation — verified by: full `package.ps1` exit 0 ("check-bundle: all checks passed", smoke PASS, 1095 files, 57.8 MiB, matching the README). The bundle's THIRD-PARTY-NOTICES.txt hash equals the committed file. test-packaging: 37 cases, 0 failed.
- [x] fmt, clippy `-D warnings`, nextest and `cargo deny check` pass — see the checks line above.

Your specific questions:
- **Notices regeneration (7 → 16 crates) is correct, neither stale nor over-inclusive.** I ran `cargo tree -p tinwave-desktop -e normal --target x86_64-pc-windows-msvc` and dropped the proc-macro subtrees. The runtime closure is exactly cxx, cxx-qt, cxx-qt-lib, foldhash (via cxx), link-cplusplus, static_assertions, thiserror, plus the 9 `windows*` crates (windows, -collections, -core, -future, -link, -numerics, -result, -strings, -threading) pulled in by `windows 0.62.2`. That is 16, matching the notices. Build-only crates (windows-sys, winapi-util, termcolor, cxx-gen, …) are correctly left out. The old file was stale: it predates the `windows` dependency (last regenerated in f25e5f0). The Apache and MIT texts for these crates are embedded in the notices as Text 18/19, so dropping `licenses\Apache-2.0.txt` loses nothing.
- **Revert spot-check holds.** I built a fixture bundle with the MIT text as `Tinwave-LICENSE-GPL-3.0.txt` and notices saying "licensed under: MIT OR Apache-2.0.", then ran both versions of check-bundle on it.
  - main's `check-bundle.ps1` + `common.ps1`: "PASS legal ... check-bundle: all checks passed", exit 0.
  - the branch's: "FAIL legal ... is not the GNU GPL version 3 text" and "FAIL legal ... does not state ... GPL-3.0-only", exit 1.
  So the new cases do fail without the change.
- **`[licenses.private] ignore = true` gap: acceptable, not blocking.** I confirmed it: a path crate `gpl-priv` with GPL-3.0-only and `publish = false` (not a workspace member) gives `licenses ok`, exit 0. The exposure is narrow:
  - deny.toml:101-104 has `unknown-git = "deny"` and no `allow-git`, so git dependencies are banned.
  - Registry crates can never be `publish = false`.
  - What remains is a path crate inside this repo that someone deliberately marks `publish = false`. That is visible in review.
  - ADR 0009 decision 5 prescribes exactly `ignore = true`, so the builder did what was decided. A guard is a proposal (below).
- **Merge with main is clean.** `git merge-tree --write-tree main t/T-0058-switch-to-gpl` exits 0. `git diff f372a9f main` touches only `work/` files, so the merged code tree equals the branch tree and the gates above apply to it.

blocking findings:
- none

design check: n/a

non-blocking findings:
- D:\dev\Tinwave-worktrees\T-0058-switch-to-gpl\apps\tinwave-desktop\packaging\package.ps1:20 — the header comment still says "-Release: D-0007 (copyright holder, source-offer contact, root LICENSE files) still pending". The root LICENSE files are no longer part of D-0007.
- D:\dev\Tinwave-worktrees\T-0058-switch-to-gpl\apps\tinwave-desktop\packaging\check-bundle.ps1:197-206 — the licence check is weak in two ways:
  - Tinwave's licence is accepted if it merely contains `GNU GENERAL PUBLIC LICENSE\s+Version 3`. A truncated or edited GPL text would pass. Comparing its hash with `$script:RepoRoot\LICENSE` would close this; test-packaging already does that, but only for the good bundle.
  - The list of rejected files is a fixed set of names. An MIT/Apache file under another name (e.g. `licenses\LICENSE-MIT.txt`) would not be caught.

proposals:
- P: Guard the `[licenses.private]` gap | why: a path crate marked `publish = false` skips the allow-list. Close it with a test asserting that every path package in `cargo metadata` is a workspace member under `crates/` or `apps/`, or by replacing `ignore = true` with per-crate `[[licenses.exceptions]]` for the tinwave-* crates (needs an ADR 0009 update) | size: S
- P: check-bundle compares `Tinwave-LICENSE-GPL-3.0.txt` byte-for-byte with the root `LICENSE` | why: the current check only matches the GPL heading | size: S

notes for lead: Merge is clean with no ordering concerns, since main's drift is work/-only. My packaging run regenerated the worktree's gitignored `target\package\tinwave-desktop-windows-x64`, identical in content to the builder's. All my scratch copies are deleted. Two processes are still running and neither is mine:
- cargo PID 41336: `nextest run --no-fail-fast -p tinwave-playback --test rt_alloc` (another agent's).
- python PID 60496: `python -I ...\Temp\t.py cmd C:/ cmake-build.cmd configure`, started 2026-11-02, a day before this review. It may be a stale leftover worth checking.
I left both alone.
