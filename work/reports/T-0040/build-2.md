HANDOFF T-0040 (round 2)
status: done
branch: t/T-0040-desktop-skeleton   commit: 6e2dba4 (round 1 was 2fb85c2)
summary: Fixed both blocking review items and added the comment-only item. The root-QML override is now debug-only, and macOS selects the `macOS` style per design-system §2.

changes:
1. Root QML URL override is debug-only:
   - In `apps/tinwave-desktop/src/app/wiring.rs`, `MAIN_QML_ENV` and the env read sit behind `#[cfg(debug_assertions)]`, via a new `main_qml_url()` helper. Release builds always load `MAIN_QML_URL`.
   - In `apps/tinwave-desktop/tests/startup.rs`, `missing_root_qml_exits_nonzero_with_a_message_instead_of_hanging` and its `read_stderr` helper are gated the same way, so `cargo test --release` passes with no warnings.
   - README says the `TINWAVE_MAIN_QML` switch is "debug builds only".
   - README and the `STYLE_ENV` doc comment in `style.rs` now say `QT_QUICK_CONTROLS_STYLE` is Qt's own documented switch, honoured in release builds too (design-system §8).
2. `apps/tinwave-desktop/src/app/style.rs`:
   - New `"macos" => "macOS"` arm with a comment "Per design-system §2. Unverified on a Mac until T-0038". `_ => "Fusion"` kept.
   - The test is renamed `macos_selects_its_native_style_and_linux_uses_fusion` and asserts macos → "macOS" and linux → "Fusion".
   - Module doc and README table row updated.
3. `apps/tinwave-desktop/qml/Sidebar.qml`: comment on the app `Menu` saying popupType is pending ADR 0007 decision 1 (Popup.Native plus the dark opt-in, falling back to Popup.Window) and is wired with the platform bridge in T-0036.

checks: fmt pass. `cargo clippy --workspace --all-targets -- -D warnings` pass in debug. `cargo clippy --release -p tinwave-desktop --all-targets -- -D warnings` pass. `cargo deny check` pass.
tests:
- Debug (`cargo test -p tinwave-desktop`): 4 unit + 2 integration passed, 0 failed.
- Release (`cargo test --release -p tinwave-desktop`): 4 unit + 1 integration passed, 0 failed (the load-failure test does not exist in release).
- I did not rerun the whole workspace test suite this round; only the desktop crate's code changed.
cleanup: no tinwave-desktop processes left running; no screenshots recaptured, per your note.

deviations / questions: none. The round-1 proposal "Decide the macOS style" is now resolved by this change and can be dropped.
