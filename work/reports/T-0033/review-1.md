REVIEW T-0033 (round 1)
verdict: APPROVE
checks (run by reviewer at 6f75257): fmt pass; clippy Windows pass (rebuilt, undocumented_unsafe_blocks applied); clippy linux-gnu pass; tests pass (tinwave-context 59 passed, 0 failed, 2 ignored — T-0014 live audio and the Credential Manager live test, neither run; workspace green); deny pass (existing duplicate-syn warning).

acceptance criteria:
- [x] Fake-store round trip; corrupt entry is a visible error, never regenerated — secret_store/tests.rs (first run, later runs, existing loaded not replaced, wrong sizes 0/1/31/33/4096 with 0 writes and entry unchanged, corrupt message names sizes never bytes in Display and Debug, unreachable store, store that forgets, loaded secret never shows bytes).
- [x] Ignored live test on a test-only target; default `cargo test` never touches the real Credential Manager — only `credential_manager_round_trip` touches it (#[ignore]); `Tinwave/test-device-secret-<16 hex>` via `with_target` (#[cfg(all(windows, test))], mod.rs:125); asserts != DEVICE_SECRET_TARGET; Drop guard deletes; the real target appears in tests only in cfg(not(windows)) `other_oses_report_unsupported`.
- [x] unsafe only in one documented module — secret_store/win.rs has 6 unsafe blocks (handoff said 5; it missed the `&*buffer.0` deref), all with SAFETY comments; lint passes; rest of the crate keeps deny(unsafe_code).
- [x] Non-Windows compiles and reports Unsupported; fmt, clippy, tests, deny.

blocking findings:
- none

answers:
- SAFETY comments true: CredFree guard (win.rs:189-191) built only after the non-null check (:208-213), not Clone ⇒ freed once; empty-blob early return (:218-219) drops it normally. CredReadW (:199-201) name NUL-terminated by `wide()` (:180-182), out-pointer on this frame. `&*buffer.0` (:214-216) and `from_raw_parts` (:221-223): copied with to_vec() before locals drop; u8 has no alignment requirement. CredWriteW (:241-244): TargetName/CredentialBlob point into live locals, size checked to fit u32, other fields zero via `..CREDENTIALW::default()`; CredWriteW copies. CredDeleteW (:253-254) test-only.
- Error mapping correct: CredReadW returns BOOL::ok() ⇒ HRESULT_FROM_WIN32(GetLastError); comparing with HRESULT::from_win32(ERROR_NOT_FOUND.0) (:203) maps exactly "no entry" to Ok(None); everything else Os.
- Corrupt handling correct: zero-length/null blob ⇒ Some(vec![]) ⇒ Corrupt{found: 0}, never "missing"; load_or_create writes only after read() returned None; errors name sizes and OS strings only; DeviceSecret Debug redacted.
- Secret generated only via the sealed CryptoRng (tinwave-core rng.rs:37-55; SystemRng only).
- CRED_PERSIST_LOCAL_MACHINE is right: per-user, later logon sessions on this machine only, no roaming. SESSION would re-key every device hash each login; ENTERPRISE would roam and make device hashes linkable across machines for no gain. Module doc (win.rs:3-4) states it correctly.
- load_or_create read-back: catches dropped writes; when both processes write before either reads back they converge; but A can read None, write a, read back a, return a, then B (which read None earlier) writes b ⇒ A's session uses a, later runs use b. The doc overclaims (non-blocking).

design check: n/a

non-blocking findings:
- mod.rs:73-75 doc says concurrent first runs "end up with the same secret" — not guaranteed; reword or serialise first-run creation (named mutex / single-instance guard). Impact small.
- win.rs:224, :230 secret bytes in Vecs not zeroed on drop (DeviceSecret neither); acceptable for a salt.
- DPAPI: a forced admin password reset can make the entry unreadable ⇒ Os or Corrupt, never a silent new secret; the start-up wiring's error text should explain it.
- Handoff miscounted unsafe blocks (6, all documented).

proposals:
- P: Single-instance guard or named mutex around the first load_or_create | size: S
- P: Wire load_or_create into app start-up; surface Corrupt/Os; never start the provider with a throwaway secret | size: S

notes for lead: scope clean (tinwave-context Cargo.toml, lib.rs, secret_store/*; Cargo.lock unchanged; only new feature Win32_Security_Credentials); no overlap with T-0024.

---
Lead (2026-11-02): merged. Doc wording → P-0078 parked nits; single-instance guard + DPAPI error text → E-01 shell/wiring (P-0093).
