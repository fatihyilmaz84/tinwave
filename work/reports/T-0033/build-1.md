HANDOFF T-0033 (round 1)
status: done
branch: t/T-0033-device-secret-keychain   commit: 6f75257
summary: New secret_store module in tinwave-context: SecretStore trait (raw read/write + provided load_or_create(&mut impl CryptoRng)); OsSecretStore on Windows Credential Manager (generic, per-user, target Tinwave/device-secret); Unsupported off Windows. Wrong-size entry => SecretStoreError::Corrupt, never overwritten. unsafe confined to secret_store/win.rs.

acceptance criteria:
- [x] Fake-store round trip; corrupt => visible error, no regeneration: the_first_run_creates_and_stores_a_secret, later_runs_load_the_same_bytes_and_write_nothing, an_existing_secret_is_loaded_not_replaced, a_wrong_size_entry_is_an_error_and_is_never_regenerated (sizes 0, 1, 31, 33, 4096; 0 writes; entry unchanged), the_corrupt_message_names_the_sizes_and_never_the_bytes, an_unreachable_store_is_an_error_not_a_throwaway_secret, a_store_that_forgets_the_new_secret_is_noticed, the_loaded_secret_never_shows_its_bytes.
- [x] Live Windows test (ignored) on a test-only target: secret_store::tests::live::credential_manager_round_trip passed once (`cargo test -p tinwave-context --lib -- --ignored secret_store`): target Tinwave/test-device-secret-<16 hex> with Drop-guard delete, asserts != real target; empty target, load_or_create twice same, corrupt 5-byte write => Corrupt with entry untouched, delete => gone. `cmdkey /list` afterwards: no leftover test credentials. Real Tinwave/device-secret never read or written; with_target is #[cfg(all(windows, test))].
- [x] unsafe only in secret_store/win.rs (#![allow(unsafe_code)]), 5 blocks with SAFETY comments (CredReadW, slice from blob, CredFree in a CredBuffer Drop guard, CredWriteW, CredDeleteW test-only); undocumented_unsafe_blocks = "deny" passes.
- [x] Non-Windows compiles + Unsupported (linux-gnu clippy --all-targets clean; other_oses_report_unsupported compiles, not run here); fmt, clippy, deny pass.

checks: fmt pass, clippy pass (Windows + Linux target), tests 393 passed / 0 failed (7 ignored incl. the live test), deny pass
files changed: crates/tinwave-context/{Cargo.toml (feature Win32_Security_Credentials), src/lib.rs (docs, pub mod secret_store, re-exports OsSecretStore, SecretStore, SecretStoreError), src/secret_store/{mod.rs, win.rs, tests.rs}}; Cargo.lock unchanged.
deviations: new src/secret_store/ module (ticket allowed); raw read/write + provided load_or_create; load_or_create reads back after the first-run write and returns the stored value (concurrent first runs agree; dropped writes are an error); tiny race window remains (no create-exclusive in Credential Manager).
proposals:
- P: Wire OsSecretStore::device_secret().load_or_create(&mut SystemRng) into app start-up; surface Corrupt to the user; never start the audio provider with a throwaway secret | size: S
- P: Fix `cargo doc -D warnings` (audio/mod.rs:4 links the private item `engine`) | size: S
- P: macOS Keychain and Linux Secret Service SecretStore backends | size: M
questions: none
