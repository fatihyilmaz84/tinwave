HANDOFF T-0015 (round 1)
status: done
branch: t/T-0015-workspace-deps-tidy   commit: 14d7a2f
summary: All ticket bullets are done in one commit on top of current main (df3f8df, no merge needed). uuid lost `v7`, `getrandom` moved to `[workspace.dependencies]`, and `CryptoRng` guards secrets from the Rng seam. Merged seeks whose net direction disagrees with the summed jumps are now suppressed and counted, and `InputSource::NAMES`/`ALL` plus the frozen list can no longer be bypassed by a new variant. One deviation: `crates/tinwave-core/src/lib.rs` touched (not in scope).

acceptance criteria:
- [x] `grep -rn "new_v7\|now_v7" crates/` empty and the workspace builds — grep exit 1; `cargo build --workspace` ok; main checked via git grep (no tinwave-playback there yet).
- [x] `cargo tree -e features -i uuid` no longer shows `v7` — remaining features default, serde, std; Cargo.lock lost one line.
- [x] A merged seek whose net movement disagrees with its jumps emits nothing and is counted — `back_nudges_that_playback_outran_at_1_5x_are_suppressed_and_counted`, `three_short_back_nudges_that_playback_outran_are_suppressed_and_counted` (0 emitted, `suppressed() == 1`). Net 0 with non-zero jump also suppressed. Existing seek tests unchanged.
- [x] `SeededRng` does not implement `CryptoRng`; `SystemRng` does — compile-time test `rng::tests::only_the_system_rng_may_generate_secrets` (ambiguity trick + `requires_crypto::<SystemRng>()`).
- [x] `git check-attr -a x.bat x.cmd` shows `eol: crlf`; jpeg, gif, webp, ttf, otf, aac, opus, sqlite, db-wal, db-shm show `binary: set`.
- [x] fmt, clippy `-D warnings`, tests, `cargo deny check` pass.

Other items:
- `SEEK_MIN_NET_DS` → `SEEK_MIN_TOTAL_DS` (coalesce.rs + lib.rs re-export).
- bus.rs `SignalDrain::wait` "Shutdown" paragraph (retained failed batch delays quit by up to one flush interval, only while the DB fails).
- coalesce.rs comment moved above its statement; module docs state the direction rule.
- id.rs doc: "RFC 9562 §6.2 Method 2, monotonic random".
- `InputSource` defined by an `input_sources!` macro (enum, `ALL`, `NAMES`, `name()` from one list; exhaustive `name()`).
- frozen.rs `frozen_input()` exhaustive match (no wildcard) returning each variant's frozen name and tag; table compared in `ALL` order; literal names/tags unchanged; Remote `[6, 200]` check untouched.
- enums.rs test `input_source_names_cover_every_variant` replaced by `input_source_names_match_the_variants`.

checks: fmt pass, clippy pass, tests 174 passed / 0 failed (core 40, signals lib 103, integration 10 + 8 + 7 + 6), deny pass; emit bench p99 200–300 ns.
files changed: .gitattributes, Cargo.toml, Cargo.lock, crates/tinwave-core/{Cargo.toml, src/rng.rs, src/id.rs, src/lib.rs}, crates/tinwave-signals/src/{enums.rs, frozen.rs, coalesce.rs, bus.rs, lib.rs}
signals added/changed: none; only behaviour change: merged seeks with to/from disagreeing with summed jumps are suppressed and counted. Schema, encoding, frozen codes unchanged.
ui: none
docs updated: none (catalog §3 already states the rule)
deviations from ticket:
- One-line re-export of `CryptoRng` in crates/tinwave-core/src/lib.rs (out of scope) so other crates can require it.
- `CryptoRng` is a plain unsealed marker (`CryptoRng: Rng`).

proposals:
- P: Make `CryptoRng` sealed, or add a SystemRng-only `secret_bytes<const N>()` helper | size: S | evidence: crates/tinwave-core/src/rng.rs

questions for lead:
- Is the lib.rs re-export acceptable?

---
Lead answers (2026-11-02): re-export accepted; seal `CryptoRng` now (private supertrait) before review (P-0056 → this
ticket); doc that secrets take `&mut impl CryptoRng`.

---
HANDOFF T-0015 (round 1, amended)
status: done
branch: t/T-0015-workspace-deps-tidy   commit: d2cbb3f (on top of 14d7a2f)
summary: CryptoRng sealed with the private-supertrait pattern in crates/tinwave-core/src/rng.rs (pub trait CryptoRng: Rng + sealed::Sealed; impl Sealed + CryptoRng for SystemRng only). Docs: secrets (e.g. device_secret) take &mut impl CryptoRng. compile_fail doc-test: an outside type cannot impl CryptoRng (verified it compiles when the seal is removed, then restored). lib.rs re-export kept.
checks: fmt pass, clippy pass, tests 174 passed + 1 doc-test, deny pass (emit bench not re-run: no emit-path change)
files changed this round: crates/tinwave-core/src/rng.rs
