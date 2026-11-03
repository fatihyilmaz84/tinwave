HANDOFF T-0058 (round 1)
status: done
branch: t/T-0058-switch-to-gpl   commit: 13e46ef
summary: The workspace licence is now GPL-3.0-only (root Cargo.toml), and deny.toml has `[licenses.private] ignore = true` with the allow-list unchanged. The packaging ships the root `LICENSE` as `licenses\Tinwave-LICENSE-GPL-3.0.txt` and no longer ships `licenses\Apache-2.0.txt`. The D-0007 check now covers only holder and contact. check-bundle also verifies Tinwave's licence files, and `THIRD-PARTY-NOTICES.txt` is regenerated and committed.

acceptance criteria:
- [x] `cargo metadata` reports GPL-3.0-only for every workspace crate; `cargo deny check` passes with the allow-list unchanged — proof: `cargo metadata --no-deps` lists tinwave-context, -core, -signals, -db, -playback, -test-alloc and -desktop as `GPL-3.0-only`. `cargo deny check` printed "advisories ok, bans ok, licenses ok, sources ok" (exit 0, only the usual duplicate-version warnings). The allow-list is not edited.
- [x] Negative check: a GPL dependency still fails `cargo deny` — proof: I added a temporary path crate `gpl-probe` (license = "GPL-3.0-only", no `publish = false`) as a dependency of tinwave-desktop. `cargo deny check licenses` printed "rejected: license is not explicitly allowed ... gpl-probe v0.1.0 <- tinwave-desktop", "licenses FAILED", exit 4. I reverted `apps/tinwave-desktop/Cargo.toml` and `Cargo.lock` with `git checkout`; the probe crate lives in the scratchpad, outside the repo.
- [x] The package contains the GPL-3.0 text as Tinwave's licence and no MIT/Apache text presented as Tinwave's licence; check-bundle still reports D-0007 as PENDING (holder, contact) and no longer lists LICENSE-MIT / LICENSE-APACHE — proof: `package.ps1 -SkipBuild` printed "PENDING d0007 ... copyright holder of Tinwave's own code ...; contact for the Qt written source offer ..." and "check-bundle: all checks passed". test-packaging asserts that `licenses\Tinwave-LICENSE-GPL-3.0.txt` has the same hash as the root `LICENSE`, that `licenses\Apache-2.0.txt` is absent, that the notices say "licensed under: GPL-3.0-only." and that the PENDING line does not match `LICENSE-MIT|LICENSE-APACHE`.
- [x] `package.ps1` and `test-packaging.ps1` exit 0; the committed `THIRD-PARTY-NOTICES.txt` matches a fresh generation — proof: `package.ps1 -SkipBuild` (with smoke test) exited 0, and a second `-SkipBuild -NoSmoke` run printed no "differs" warning (the script compares file hashes). `test-packaging.ps1` ran 37 cases, 0 failed.
- [x] fmt, clippy `-D warnings`, nextest, `cargo deny check` pass — proof: fmt exit 0, clippy exit 0, nextest 622 passed / 0 failed / 6 skipped, deny ok.

checks: fmt pass, clippy pass, tests 622 passed / 0 failed (6 skipped), deny pass

revert test: I temporarily put HEAD's `check-bundle.ps1` back and re-ran test-packaging. The 4 new cases failed: missing GPL file, MIT text as Tinwave's licence, Apache-2.0.txt shipped, notices naming another licence. The result was 37 cases, 4 failed. I then restored the new file. The reworked D-0007 cases (identity filled in, holder only) assert behaviour that the old `common.ps1` could not produce, because it always listed LICENSE-MIT as pending.

files changed (all under D:\dev\Tinwave-worktrees\T-0058-switch-to-gpl):
- Cargo.toml
- deny.toml
- apps\tinwave-desktop\README.md
- apps\tinwave-desktop\packaging\package.ps1 (copies root `LICENSE`, throws if it is missing)
- apps\tinwave-desktop\packaging\common.ps1 (D-0007 pending list is now holder and contact only)
- apps\tinwave-desktop\packaging\check-bundle.ps1
- apps\tinwave-desktop\packaging\New-ThirdPartyNotices.ps1
- apps\tinwave-desktop\packaging\test-packaging.ps1
- apps\tinwave-desktop\packaging\README.md
- apps\tinwave-desktop\packaging\THIRD-PARTY-NOTICES.txt

signals added/changed: none
ui: none
docs updated: apps/tinwave-desktop/packaging/README.md, apps/tinwave-desktop/README.md (licence/packaging lines only)

deviations from ticket:
- I added more than the ticket listed. `check-bundle`'s `legal` check now requires the GPL-3.0 text, rejects a bundle that carries `licenses\Apache-2.0.txt` or `Tinwave-LICENSE-MIT/-APACHE.txt`, and requires the notices to state GPL-3.0-only. `New-ThirdPartyNotices.ps1` throws if the package licence is not `GPL-3.0-only`, because it points at the GPL text. All of this is inside the packaging scope.
- The regenerated `THIRD-PARTY-NOTICES.txt` is a much larger diff than a licence-line change (+272 lines). The previously committed file was stale: it listed 7 crates, and a fresh generation on main gives 16, because the `windows*` crates joined the closure through other tickets. It now matches the current build.
- `license-texts\Apache-2.0.txt` stays. The generator still uses it for crates that ship no licence file (cxx-qt) and for licences under "MIT OR Apache-2.0". It is no longer copied into the bundle as `licenses\Apache-2.0.txt`.
- The `deny.toml` negative check only covers a crate that cargo-deny does not treat as private. A path crate with `publish = false` that is GPL would be ignored by `[licenses.private]`. Registry crates are never private, so the closed-source-safety goal holds for third-party dependencies.

proposals:
- P: Release checklist item for the GPL source offer | why: ADR 0009 requires the way to get the exact corresponding source (repository at the release tag) and an About view; the packaging checks neither (T-0054 owns the About view) | size: S | evidence: docs/adr/0009-project-licence.md "Consequences"
- P: A `check-bundle` d0007 item for the "corresponding source" location once D-0007 names the holder and contact | why: GPL-3.0 §6 needs it in the release notices, and nothing in `release-identity.json` carries it yet | size: S | evidence: apps/tinwave-desktop/packaging/release-identity.json

questions for lead:
- None. D-0007 stays PENDING (null holder and contact); nothing was invented. Two `cargo` processes (PIDs 58112, 66428) were still running at the end. They are not mine: all my cargo commands finished, and the other two builders are working in parallel. I left them alone. `target\package\test` fixtures are kept inside the worktree's gitignored `target\`.
