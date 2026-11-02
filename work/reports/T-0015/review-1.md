REVIEW T-0015 (round 1)
verdict: APPROVE
checks (run by reviewer, worktree D:\dev\Tinwave-worktrees\T-0015-workspace-deps-tidy @ d2cbb3f, clean status): fmt pass; clippy `--workspace --all-targets -- -D warnings` pass; tests 174 passed / 0 failed (core 40, signals lib 103, integration 10 + 8 + 7 + 6) + 1 doc-test (`rng::CryptoRng` compile_fail); deny pass; emit bench gate exit 0 (p99 200 ns in all three scenarios).

acceptance criteria:
- [x] `grep -rn "new_v7\|now_v7" crates/` empty and workspace builds — grep exit 1 on this branch; `git grep -n "new_v7\|now_v7" t/T-0016-playback-engine-core -- crates/` also exit 1 (tinwave-playback uses no `Uuid::`), so merge order doesn't matter.
- [x] `cargo tree -e features -i uuid` shows only default, serde, std; Cargo.lock drops the getrandom edge under uuid.
- [x] Merged seek disagreeing with its jumps emits nothing and is counted — coalesce.rs:881 and following (`back_nudges_that_playback_outran_at_1_5x_are_suppressed_and_counted` 100.0→100.65; `three_short_back_nudges_that_playback_outran_are_suppressed_and_counted` 100.0→101.4); both assert `flush_all()==0`, no events, `suppressed()==1`. Rule at coalesce.rs:227-240: summed jumps for > 2 s first, then strict sign(jumps) == sign(to−from) (net 0 also suppressed). Matches catalog SeekBack row (signal-catalog.md:56); hidden-by-playback test still emits SeekBack.
- [x] `SeededRng` does not implement `CryptoRng`; `SystemRng` does — `rng::tests::only_the_system_rng_may_generate_secrets`; external scratch crate: `impl CryptoRng for Constant {}` fails E0277 on `tinwave_core::rng::sealed::Sealed` (rustc "sealed trait" note), passing `SeededRng` fails E0277, naming `sealed::Sealed` fails E0603. `SystemRng` is the only OS-random touchpoint (rng.rs:50 `getrandom::fill`).
- [x] `git check-attr`: x.bat / x.cmd text set, eol crlf; jpeg, gif, webp, ttf, otf, aac, opus, sqlite, db-wal, db-shm binary set; no tracked file matches the new patterns; scratch clone `git add --renormalize .` ⇒ 0 porcelain lines.
- [x] fmt, clippy, tests, deny pass.

Other items verified: getrandom in [workspace.dependencies] (Cargo.toml:18), tinwave-core `{ workspace = true }`; `SEEK_MIN_TOTAL_DS` rename complete; bus.rs:573-576 "Shutdown" paragraph accurate; id.rs:78 "RFC 9562 §6.2 Method 2, monotonic random"; coalesce.rs:46-49 direction rule; approved `CryptoRng` re-export lib.rs:17; all touched files in scope. InputSource macro (enums.rs:222-276): enum, ALL, NAMES, name() from one list; exhaustive `name()`; wrong/duplicated sample fails `input_source_names_match_the_variants`; `frozen_input()` (frozen.rs:239-251) exhaustive; frozen literals unchanged (Mouse 0 … System 7), Remote `[6, 200]` check untouched; declaration order (serde indices) unchanged.

blocking findings:
- none

design check: n/a

non-blocking findings:
- coalesce.rs:870 and :886: the moved comment still reads as a continuation of the previous line's trailing comment (rustfmt column-aligns it); the new test at :886 repeats the pattern. Put the standalone comment before `rig.clock.advance(..)` or drop the trailing comment above.
- Frozen-list robustness unchanged (a deliberate rename of both variant and literal still passes, as before). Acceptable.

proposals:
- none

notes for lead:
- Merge order with T-0016: both edit the same `[workspace.dependencies]` block; T-0016 still has uuid `["v7", "serde"]` and crate-local `getrandom = "0.4"` in tinwave-core from its base. Expect Cargo.toml/Cargo.lock conflicts for whichever lands second; keep `uuid = { version = "1", features = ["serde"] }`, workspace getrandom, T-0016's new deps; regenerate Cargo.lock.
- T-0016 uses neither `SEEK_MIN_NET_DS`, `InputSource::NAMES` nor uuid v7.
- The bench contention run reported 202,985 reserved overflows (T-0022 review saw 0); the bench text calls this expected for a tight loop at ~10^6 events/s; T-0015 doesn't touch the emit path; read as machine-load variance. p99 gate passed.
- Scratch artifacts in the session scratchpad (sealcheck/, renorm/).

---
Lead (2026-11-02): merged. Cosmetic comment finding parked (P-0058, fix when the file is next touched). Reserved-overflow
count in the contention bench checked by the lead on main (see T-0015 merge note).
