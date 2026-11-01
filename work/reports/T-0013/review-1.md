REVIEW T-0013 (round 1)
verdict: APPROVE
checks (run by reviewer, worktree D:\dev\Tinwave-worktrees\T-0013-injectable-clock @ e925140, clean status): fmt pass (exit 0), clippy pass (`--workspace --all-targets -- -D warnings`, exit 0), tests 39 passed / 0 failed (`cargo test --workspace`, exit 0), deny pass ("advisories ok, bans ok, licenses ok, sources ok", exit 0)

acceptance criteria:
- [x] grep `now_v7|SystemTime::now|Instant::now` in crates/tinwave-core/src only matches `SystemClock`. Verified by my own grep: there is exactly one hit, `crates/tinwave-core/src/clock.rs:35`, inside `impl Clock for SystemClock`. OS randomness is also confined to `SystemRng` (rng.rs:21, `getrandom::fill`).
- [x] Ids from a `ManualClock` embed exactly its time, and advancing the clock orders them. Verified by `ids_from_a_manual_clock_embed_exactly_its_time` and `advancing_the_clock_orders_ids` (id.rs:363, 373). The old real-time sleep test is gone.
- [x] Ids at the same `ManualClock` instant are unique and strictly increasing. Verified by `ids_at_one_instant_are_unique_and_strictly_increasing` (id.rs:387: 1000 ids, windows strictly `<`, HashSet size, all `created_at == T0`). Related: `a_backwards_clock_never_breaks_ordering`, `random_bit_exhaustion_rolls_into_the_next_millisecond`, `exhaustion_at_the_last_representable_millisecond_is_an_error`.
- [x] Same clock time plus same random bits gives the identical id, byte for byte. Verified by `same_time_and_same_random_bits_give_identical_ids` (id.rs:446), which pins all 16 bytes. I checked the expected bytes by hand: 0xAB→0x7B, 0xEF→0x80|0x2F. Also `generators_with_equal_clock_and_seed_produce_identical_ids`.
- [x] No id type implements `Default`. Verified by the compile-time ambiguity check `id_types_do_not_implement_default` (id.rs:330-343) over all 5 id types; the technique is sound. A grep for `Default` in src finds only docs and that test.
- [x] fmt, clippy, tests and deny pass. Verified by my own runs above.

Specific points the lead asked about:
- UUIDv7 layout (id.rs:62-72) is correct per RFC 9562 §5.7:
  - Bytes 0-5 hold the 48-bit big-endian ms (`to_be_bytes()[2..]`).
  - Byte 6 is `0x70 | rand_a[11:8]` and byte 7 is `rand_a[7:0]`.
  - Byte 8 is `0x80 | rand_b[61:56]` (variant 10) and bytes 9-15 are `rand_b[55:0]`.
  - `Rand74::from_bytes` masks exactly the 6 reserved bits.
  - The derived `Ord` on `Rand74 {a, b}` matches the UUID byte order, so `successor()` (b+1, carrying into a, `None` at all-ones) gives strictly larger UUIDs.
  - A stalled or backwards clock (`now <= last_ms`) reuses `last_ms` and increments.
  - On exhaustion it moves to `last_ms + 1` with fresh bits (RFC §6.2 allows advancing the timestamp ahead of real time). At `MAX_V7_MILLIS` it returns `TimestampOutOfRange(2^48)`. `last_ms + 1` cannot overflow because `last_ms <= 2^48-1`.
  - One labeling nit, below: this is §6.2 Method 2 (monotonic random, increment 1), not Method 1.
- `SystemClock` pre-1970 handling (clock.rs:35-39) cannot panic. `try_from` falls back to `i64::MAX`/`i64::MIN`, and `-m` is safe because `m <= i64::MAX`. A negative or over-range value is rejected by `v7_millis` (id.rs:52-59) as `TimestampOutOfRange`, which `generate` returns as an `Err`. Tested by `generate_rejects_a_clock_outside_the_uuid_range`.
- `SeededRng` is acceptable as is:
  - Production code takes its rng from `IdGenerator::system()` → `SystemRng` (id.rs:109-111).
  - The name says "Seeded", and the doc says "not cryptographic and must not be used for secrets" (rng.rs:25-28).
  - No secret generation exists in the tree yet. `device_secret` is only mentioned in docs (privacy-and-signals.md:38, ADR 0004:81).
  - The residual risk is a future secret generator taking `&mut impl Rng`; see proposals.
- `IdGenerator` with `&mut self` and no internal sync is acceptable under rust.md Concurrency:
  - `generate` is synchronous and O(1), with at most one getrandom call per fresh millisecond.
  - If shared, it goes behind a short `Mutex` that is never held across `.await`, or it is owned by one task.
  - The signal emit hot path uses `SignalEventId(device, seq)`, not UUIDs, so the generator is not on the emit path or any audio callback.
  - Ordering is documented as per generator only (id.rs:76). Separate generators remain collision-safe through fresh 74-bit randomness.
- Deviations: I judge all of them acceptable.
  - Removing `new`/`new_at`/`Default` is what the ticket asked for.
  - `from_parts` plus the in-crate `Rng` trait (no rand_core) keeps the crate dependency-light.
  - `ManualClock`/`SeededRng` are always compiled; this was the builder's call and is recorded.
  - Writing UUIDs by hand is needed because uuid's `new_v7` cannot take injected randomness.
  - The Cargo.lock edge is accepted standing practice; getrandom 0.4.3 was already in the tree.

blocking findings:
- none

design check: n/a

non-blocking findings:
- crates/tinwave-core/src/id.rs:78: the doc says "RFC 9562 method 1". The algorithm increments the full 74 random bits, seeded randomly at each new ms, by 1. That is §6.2 Method 2 (Monotonic Random); increment 1 is permitted when guessability is not a concern. Method 1 is a fixed-length dedicated counter, recommended at 12-42 bits. Fix the label the next time this file is touched.
- crates/tinwave-core/Cargo.toml:11: `getrandom = "0.4"` is declared crate-locally, against rust.md:8 (workspace-level versions). The builder could not fix it because the root Cargo.toml is out of scope, and the builder's own proposal covers it.
- crates/tinwave-core/src/id.rs:114: `v7_millis(self.clock.now())?` runs before the monotonic branch. A clock that jumps below 1970 after valid ids returns an error instead of continuing from `last_ms`. This is acceptable because the error is visible, not silent, and the scenario is unrealistic. I mention it only so the behaviour is known.
- `tinwave-core` is documented as "No IO" (architecture.md:7) and now makes the clock and getrandom syscalls in `SystemClock`/`SystemRng`. The ticket explicitly sanctions this, so it is not a finding against the branch. The architecture line could say "no IO beyond the system clock/RNG seams".

proposals:
- P: Guard secrets from the `Rng` seam: document on `trait Rng` that secrets (e.g. `device_secret`) must use `SystemRng`/getrandom directly or a separate `CryptoRng` marker trait that `SeededRng` does not implement | why: `IdGenerator::new(SystemClock, SeededRng::new(0))` or a future `fn make_secret(&mut impl Rng)` would compile with the deterministic generator | size: S
- P: Drop the uuid `v7` feature and move getrandom into `[workspace.dependencies]` (the builder's two proposals together) | why: dead getrandom path in uuid; rust.md:8 | size: S. Check T-0004 for `Uuid::new_v7` first.

notes for lead: Safe to merge. T-0004 must switch from `X::new()` to `X::generate(&mut IdGenerator)` / `from_parts` after this lands, as you noted. I did not edit, build artifacts aside, or commit anything in the worktree, and its status is clean at e925140.
