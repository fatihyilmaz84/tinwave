HANDOFF T-0013 (round 1)
status: done
branch: t/T-0013-injectable-clock   commit: e925140
summary: Added `Clock` (`SystemClock`, `ManualClock`) and `Rng` (`SystemRng`, `SeededRng`) seams to tinwave-core. Id generation now goes through a new `IdGenerator` that owns a clock and an rng and produces strictly increasing UUIDv7s. `new()`, `new_at()` and `Default` are removed from the id types. `SystemTime::now` now appears only in `SystemClock`.

acceptance criteria:
- [x] grep for `now_v7|SystemTime::now|Instant::now` in crates/tinwave-core/src matches only SystemClock — proof: the grep output is exactly one line, `crates/tinwave-core/src/clock.rs:35: let ms = match SystemTime::now().duration_since(UNIX_EPOCH)`, inside `impl Clock for SystemClock`.
- [x] Ids from a ManualClock embed exactly its time, and advancing the clock orders ids — proof: `ids_from_a_manual_clock_embed_exactly_its_time`, `advancing_the_clock_orders_ids`. The old real-time sleep test is converted to ManualClock (no sleeping left).
- [x] Ids at the same ManualClock instant are unique and strictly increasing — proof: `ids_at_one_instant_are_unique_and_strictly_increasing` (1000 ids). Related tests: `ids_of_different_types_share_one_increasing_sequence`, `a_backwards_clock_never_breaks_ordering`, `random_bit_exhaustion_rolls_into_the_next_millisecond`, `exhaustion_at_the_last_representable_millisecond_is_an_error`.
- [x] Same clock time and same random bits give an identical id, byte for byte — proof: `same_time_and_same_random_bits_give_identical_ids` (also pins the exact 16-byte layout) and `generators_with_equal_clock_and_seed_produce_identical_ids`.
- [x] No id type implements `Default` — proof: compile-time test `id_types_do_not_implement_default`, which covers all 5 id types. I checked it can fail: swapping in `u32` (which has `Default`) gives E0283, then I restored the file.
- [x] fmt, clippy `-D warnings`, tests and `cargo deny check` pass — proof: see checks line.

checks: fmt pass, clippy pass, tests 39 passed / 0 failed (`cargo test --workspace`), deny pass ("advisories ok, bans ok, licenses ok, sources ok")
files changed:
- crates/tinwave-core/src/clock.rs (new)
- crates/tinwave-core/src/rng.rs (new)
- crates/tinwave-core/src/id.rs
- crates/tinwave-core/src/lib.rs
- crates/tinwave-core/src/error.rs
- crates/tinwave-core/Cargo.toml
- Cargo.lock (outside the literal scope; see deviations)
signals added/changed: none
ui: none
docs updated: none (no doc under `docs/` named this API)

new public API (all re-exported from `tinwave_core`):
- `trait Clock: Send + Sync { fn now(&self) -> UtcMillis }`, with blanket impls for `Arc<C>` and `&C`.
- `SystemClock`, the only system-time reader.
- `ManualClock`: `new(UtcMillis)`, `set(UtcMillis)`, `advance(i64)`. It is Clone, clones share the same time, it is thread-safe, and `advance` saturates.
- `trait Rng: Send { fn fill_bytes(&mut self, &mut [u8]) -> Result<(), CoreError> }`.
- `SystemRng` (getrandom-backed, the only OS-random touchpoint) and `SeededRng::new(u64)` (SplitMix64, deterministic, not cryptographic).
- `IdGenerator`: `new(impl Clock + 'static, impl Rng + 'static)` and `system()`. Ids are strictly increasing per generator across all id types. A non-advancing or backwards clock reuses the last millisecond and increments the 74 random bits (RFC 9562 method 1). Overflow rolls into the next millisecond.
- On each of `RecordingId`, `SourceId`, `SessionId` and `DeviceId`:
  - `X::generate(&mut IdGenerator) -> Result<X, CoreError>`
  - `X::from_parts(UtcMillis, [u8; 10]) -> Result<X, CoreError>` (pure and deterministic; 6 of the 80 bits are dropped for version and variant)
- `CoreError::RandomUnavailable(String)`.

deviations from ticket (removed or renamed public items, for T-0004):
- Removed `X::new()` on `RecordingId`, `SourceId`, `SessionId` and `DeviceId`. Use `X::generate(&mut generator)`. In production the generator is `IdGenerator::system()`. In tests it is `IdGenerator::new(ManualClock::new(..), SeededRng::new(..))`.
- Removed `X::new_at(UtcMillis)`. It drew OS randomness outside `SystemRng`. Use `X::from_parts(at, [u8; 10])` for a pinned id, or `generate` with a `ManualClock`.
- Removed `impl Default` on the four uuid ids. `SignalEventId` never had one.
- Kept and unchanged: `from_uuid`, `as_uuid`, `as_bytes`, `from_bytes`, `created_at`, `SignalEventId::new(device, seq)`, Display/FromStr/serde.
- Added `getrandom = "0.4"` as a direct dependency of tinwave-core only (not in `[workspace.dependencies]`, because that file is outside scope). It was already in the tree through uuid at 0.4.3. The only Cargo.lock change is that one dependency edge under tinwave-core, with no new crates. Cargo.lock is outside the literal scope, but the T-0001 review already said to add it.
- Ids are now assembled by hand with `Uuid::from_bytes`, because uuid's `new_v7` has no injectable randomness. The workspace `uuid` `v7` feature is no longer needed; I left it alone because the root Cargo.toml is out of scope.
- Decision on the Rng seam: a plain `[u8; 10]` constructor plus a small in-crate `Rng` trait, with no `rand_core` dependency. It needs no new dependency, `deny` passes, and `from_parts` takes pure bytes.
- Decision on test clocks: `ManualClock` and `SeededRng` are always compiled, with no `test-clock` feature. They are tiny and dependency-free, and other crates' dev-dependencies can use them with no feature wiring.
- Mutable state: `IdGenerator` takes `&mut self`, so shared use across threads needs a `Mutex` or an owner task. This keeps the per-id cost low and avoids a poisoning policy inside the crate.
- `SystemClock` reports a system clock set before 1970 as negative milliseconds instead of clamping it. It saturates at the `i64` limits.

proposals:
- P: Drop the `v7` feature from the workspace `uuid` dependency | why: tinwave-core no longer calls `Uuid::new_v7` or `now_v7`, so uuid's own RNG/getrandom path is dead weight | size: S | evidence: Cargo.toml:19 (`features = ["v7", "serde"]`); the `uuid` entry in Cargo.lock lists `getrandom`. Check T-0004 first in case it uses `Uuid::new_v7`.
- P: Move `getrandom` into `[workspace.dependencies]` | why: single place for versions once a second crate needs OS randomness | size: S | evidence: crates/tinwave-core/Cargo.toml:11

questions for lead:
- none
