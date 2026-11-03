# Board

_Maintained by the lead. Last updated: 2026-11-02._

## Needs the PO
- [D-0008](decisions/D-0008-canvas-vs-tokens.md): should the design artboards match the token colours exactly, and
  keep 20 px sidebar icons on macOS/Linux too? (recommendation: yes and yes; not blocking)
- [D-0007](decisions/D-0007-licence-holder-and-source-offer.md): who is the copyright holder in Tinwave's own MIT/Apache
  texts, and what contact goes in the Qt source offer? Blocks a public release only.
- **Check your system volume.** T-0008 left master volume at 2 % unmuted; T-0003 later found the default output
  muted at 6 %. Set it the way you want before any listening test.
- Runbooks, when convenient (they gate acceptance, not current work):
  [T-0011 PO-RUNBOOK](../spikes/T-0011-native-styles/PO-RUNBOOK.md) (live OS accent/theme/reduced motion/high contrast/transparency/text size),
  [T-0008 PO-RUNBOOK](../spikes/T-0008-win-volume/PO-RUNBOOK.md) R1–R3, R5–R9 and
  [T-0003 PO-RUNBOOK](../spikes/T-0003-audio-backend/PO-RUNBOOK.md) sections 1, 2, 4c, 5.
  T-0011 runbook now runs against the real app (T-0036): also check that the native app menu turns dark when
  you switch Windows from light to dark while Tinwave runs with the system scheme.
- **Listening pass (ADR 0006):** run `cargo run --release -- quality wav` in `spikes/T-0009-time-stretch/`
  and listen to `out/*.wav` — are the softer hi-hats at 0.75× and the ~15 ms dip per rate step OK for v1?
  Only a "no" would bring up Rubber Band (GPL/commercial licence decision).
- Optional cleanup: the T-0009 spike extracted `libclang.dll` (PyPI `libclang` 18.1.1 wheel, LLVM) into temp
  folders only (`%TEMP%\claude\libclang_pkg` and the session scratchpad); nothing installed or committed.
- For T-0020: install ffmpeg (or allow the agent to) and point to a folder with a few real M4A albums and other formats.
- Delete leftovers: `_move-into-.claude/`, `Claude outputs/`, and
  `D:\dev\Tinwave-worktrees\` folders `T-0008-spike-windows-volume-device`, `T-0002-spike-cxx-qt`,
  `T-0040-desktop-skeleton`, `T-0024-playback-controller` (all merged; Windows refused the delete; their build
  output was removed by the lead on 2026-11-03 to free 14 GB — D: had 10 GB free).

## In progress
_none_

Worktrees are made by the lead with `git worktree add` under `D:\dev\Tinwave-worktrees\` (the Agent tool's built-in
worktree isolation fails on this machine: the `^{commit}` ref loses its braces).

## In review
| Ticket | Reviewer | Branch / worktree |
|---|---|---|
| [T-0019](tickets/T-0019-resampler-and-dither.md) Resampler (rubato) + exclusive-mode dither (E-01) | tw-reviewer r1 (build-1: continuous same-rate, TPDF on int outputs, 595 tests) | `t/T-0019-resampler-and-dither` @ b89cb0b · `D:\dev\Tinwave-worktrees\T-0019-resampler-and-dither` |
| [T-0053](tickets/T-0053-platform-bridge-followups.md) Platform bridge follow-ups (P2) | tw-reviewer r2 (build-2: portable headless rule, pinned switch, FreeLibrary on miss) | `t/T-0053-platform-bridge-followups` @ 1aa919f · `D:\dev\Tinwave-worktrees\T-0053-platform-bridge-followups` |
| [T-0052](tickets/T-0052-capture-tool-improvements.md) capture.ps1: desktop crop, click/key steps, cursor restore (P2) | tw-reviewer r1 (build-1: -Mode desktop, @click/@down/@up/@wait, readiness) | `t/T-0052-capture-tool-improvements` @ cd3d5e8 · `D:\dev\Tinwave-worktrees\T-0052-capture-tool-improvements` |

## Ready (dispatch order)
| Ticket | Type | Size | Scope | Note |
|---|---|---|---|---|
| [T-0055](tickets/T-0055-canvas-prose-and-render-harness.md) Canvas prose + local render harness (P2) | chore | S | docs/design | free |
| [T-0027](tickets/T-0027-rate-control-engine.md) Rate control in the engine (E-01) | feature | M | tinwave-playback | after T-0019 (same engine paths) |
| [T-0025](tickets/T-0025-flush-fade.md) Fade on seek/skip flush (P2) | feature | S | tinwave-playback | free (T-0026 merges first if it lands) |

Max 3 agents in flight, reviewers included. Root `Cargo.toml` [workspace.dependencies] and `Cargo.lock` don't count as
overlap (work/README.md); the branch merging second merges main first.

## Blocked
| Ticket | Waiting for |
|---|---|
| [T-0037](tickets/T-0037-qt-610-verification.md) Re-run T-0011 harness on Qt 6.10 | PO: optional Qt 6.10 MSVC kit |
| [T-0038](tickets/T-0038-spike-macos-native-styles.md) Spike: macOS native styles | macOS hardware (D-0001) |
| [T-0039](tickets/T-0039-spike-linux-native-styles.md) Spike: Linux native styles | Linux machine/CI (D-0001) |
| [T-0020](tickets/T-0020-spike-format-corpus.md) Spike: real-world format corpus | PO: ffmpeg + real files |

## Backlog
| Ticket | Depends on |
|---|---|
| [T-0057](tickets/T-0057-exclusive-reopen-at-file-rate.md) Exclusive mode reopens at the file rate (P1) | T-0019 |
| [T-0056](tickets/T-0056-resampler-late-append.md) Resampler continuity across a late append (P2) | T-0019 |
| [T-0054](tickets/T-0054-release-readiness-packaging.md) Release-readiness packaging checks (P2) | T-0051, D-0007 |
| [T-0018](tickets/T-0018-gapless-metadata-scanner.md) Gapless metadata in scanner (E-01) | T-0016, scanner ticket |
| [T-0010](tickets/T-0010-spike-jellyfin.md) Spike: Jellyfin | — (P2) |

## Design gaps to close before E-01 screen tickets
The canvas covers Library (Windows), Album + Queue (macOS), Smart Radio (Linux) and Signal Center (Windows).
E-01 also lists **Artist**, **Now Playing**, Import/scan progress and Settings, which have no artboard yet —
those screen tickets cannot be `ready` until the PO provides an artboard or direction (work/README.md DoR).

## Decisions & design
Design system adopted 2026-10-29: [docs/design/design-system.md](../docs/design/design-system.md), ADR 0003.
Decided 2026-11-01: [D-0001](decisions/D-0001-primary-platform.md) Windows first ·
[D-0002](decisions/D-0002-epic-order.md) recommendations before sync/connectors (spec §16 updated).
Accepted 2026-11-01: [ADR 0004](../docs/adr/0004-windows-volume-device-provider.md) Windows volume/device provider.
Decided 2026-11-02: [D-0003](decisions/D-0003-audio-stack-and-mpl.md) audio stack, MPL scoped to symphonia* ·
[D-0004](decisions/D-0004-aac-decode-route.md) AAC route at the Phase 9 license review.
Decided 2026-11-02: [D-0005](decisions/D-0005-t0016-third-round.md) merge T-0016 now, fix in T-0028.
Accepted 2026-11-02: [ADR 0005](../docs/adr/0005-audio-backend.md) audio backend (spec §2/§7 updated) ·
[ADR 0006](../docs/adr/0006-time-stretch.md) time-stretch (Signalsmith, pending PO listening pass).
Accepted 2026-11-02: [ADR 0008](../docs/adr/0008-cxx-qt-bridge.md) CXX-Qt 0.10, Cargo-driven, dynamic Qt.
Decided 2026-11-02: [D-0006](decisions/D-0006-native-style-findings.md) native menus + dark fix, Qt themed confirmations,
Qt floor 6.10, memory = private working set → [ADR 0007](../docs/adr/0007-native-style-findings.md) (amends 0003).

## Done
| Ticket | Merged | Reports |
|---|---|---|
| [T-0001](tickets/T-0001-workspace-and-core-ids.md) Workspace, gates, core ids | 1452c72 | build-1/2, review-1 (changes), review-2 (approve) |
| [T-0008](tickets/T-0008-spike-windows-volume-device.md) Spike: Windows volume/device | 1479562 | spike-1 → ADR 0004 |
| [T-0013](tickets/T-0013-injectable-clock.md) Injectable Clock + Rng seam | 0f41479 | build-1, review-1 (approve) |
| [T-0012](tickets/T-0012-gitattributes.md) .gitattributes, LF endings | d43431e | build-1, review-1 (approve) |
| [T-0003](tickets/T-0003-spike-audio-backend.md) Spike: audio backend | 2edd5dd | spike-1 → D-0003, D-0004, ADR 0005 |
| [T-0004](tickets/T-0004-signal-schema-codec.md) Signal schema & codec | e6d2cb6 | build-1/2, review-1 (changes), review-2 (approve) |
| [T-0005](tickets/T-0005-signal-bus.md) Signal bus | ee20850 | build-1, review-1 (approve) |
| [T-0022](tickets/T-0022-signal-bus-hardening.md) Signal bus hardening | a8b64dc | build-1, review-1 (approve) |
| [T-0015](tickets/T-0015-workspace-deps-tidy.md) Tidy: deps, sealed CryptoRng, .bat/.cmd, InputSource, seek rule | 85c7012 | build-1, review-1 (approve) |
| [T-0016](tickets/T-0016-playback-engine-core.md) Playback engine core | b150fe6 | build-1/2, review-1/2 (changes); merged per D-0005, fixes in T-0028 |
| [T-0014](tickets/T-0014-windows-volume-device-provider.md) Windows output-device + system-volume providers | f831e27 | build-1/2, review-1 (changes), review-2 (approve) |
| [T-0046](tickets/T-0046-canvas-token-sync.md) Canvas token sheets synced with §3.1, nav icons 20 px | MERGE | build-1, review-1 (approve) |
| [T-0026](tickets/T-0026-signalsmith-wrapper.md) Vendored Signalsmith Stretch wrapper | 26d5319 | build-1, review-1 (approve) |
| [T-0045](tickets/T-0045-contrast-selfcheck-test.md) Theme.selfCheck + TwIcon probe as committed tests | c5667c4 | build-1/1b, review-1 (approve); 571 tests |
| [T-0023](tickets/T-0023-live-queue-edits.md) Live queue edits + queue signals, deferred no-session acts | 5d4742b | build-1/1b/2, review-1 (changes), review-2 (approve); 569 tests |
| [T-0051](tickets/T-0051-packaging-hardening.md) Packaging hardening + notices completeness | d6407a7 | build-1, review-1 (approve); 31 packaging cases |
| [T-0036](tickets/T-0036-windows-platform-bridge.md) Windows platform bridge (motion, contrast, Mica, dark menus, --accent) | 6d12d44 | build-1, review-1 (approve); 543 tests |
| [T-0050](tickets/T-0050-nav-focus-ring.md) Sidebar keyboard ring follows the current row; Home/End/Page | fc1bfc8 | build-1/2, review-1/2 (changes); round-3 fix by lead 3b60319 |
| [T-0049](tickets/T-0049-artwork-provider-followups.md) Artwork follow-ups + cancel-storm flake fix | 964adad | build-1, review-1 (approve) |
| [T-0042](tickets/T-0042-windows-packaging-notices.md) Windows packaging + THIRD-PARTY-NOTICES | 7ae748f | build-1, review-1 (approve), notices regenerated; 57.7 MiB bundle |
| [T-0043](tickets/T-0043-icon-pipeline.md) Icon pipeline: Lucide SVGs, TwIcon, capture helper | 59a03f8 | build-1/2, review-1 (changes), review-2 (approve) |
| [T-0048](tickets/T-0048-growth-barrier-no-spin.md) Growth harness: no spin, per-profile CPU budget, hang backstop | 3a87d77 | build-1, review-1 (approve); 495 tests |
| [T-0044](tickets/T-0044-artwork-image-provider.md) Artwork image provider: T-0041 pattern, RAII UI owner | 1486177 | build-1/2, review-1 (changes), review-2 (approve); 1,960 stress runs, 0 crashes |
| [T-0047](tickets/T-0047-storage-test-time-budget.md) storage_growth speed check on writer CPU time | d4a2ad6 | build-1, review-1 (approve); 480 tests on main |
| [T-0034](tickets/T-0034-playstarted-when-heard.md) PlayStarted when heard; initial StreamInfo; initial_repeat | 0122962 | build-1/2, review-1/2 (changes); round-3 fix by lead a1e0888; 480 tests |
| [T-0035](tickets/T-0035-design-token-fixes.md) Design-system §3 token fixes + shell tokens | 11bc7e1 | build-1/2, review-1/2 (changes); round-3 fix by lead 481582b |
| [T-0024](tickets/T-0024-playback-controller.md) Playback controller → T0 signals, sessions | f1abd7b | build-1/1b/2, review-1 (changes), review-2 (approve); 464 tests on main |
| [T-0040](tickets/T-0040-desktop-skeleton.md) Desktop skeleton: CXX-Qt crate, Fluent shell, Theme | 511c121 | build-1/2, review-1 (changes), review-2 (approve); 312 ms warm, 39 MB PWS |
| [T-0032](tickets/T-0032-settings-followups.md) Signal settings follow-ups | 48b97ad | review-1 APPROVE |
| [T-0041](tickets/T-0041-image-provider-crash.md) Image-provider crash investigation | dcb1464 | spike-1 → ADR 0008 addendum |
| [T-0002](tickets/T-0002-spike-cxx-qt.md) Spike: CXX-Qt bridge | f5ebab3 | spike-1 → ADR 0008 |
| [T-0011](tickets/T-0011-spike-native-qt-styles.md) Spike: native Qt styles & Theme | 1613da8 | spike-1 → ADR 0007, D-0006 |
| [T-0033](tickets/T-0033-device-secret-keychain.md) DeviceSecret in Windows Credential Manager | 8612674 | build-1, review-1 (approve) |
| [T-0021](tickets/T-0021-persist-signal-settings.md) Persist Signal Center toggles (fail closed) | 6c24c4e | build-1, review-1 (approve) |
| [T-0031](tickets/T-0031-engine-boundary-followups.md) Engine boundary follow-ups | 0bf81c2 | build-1, review-1 (approve) |
| [T-0007](tickets/T-0007-storage-growth-test.md) Storage growth test v1 | fb8548e | build-1/2, review-1 (changes), review-2 (approve); 4.38 MB / 90 days |
| [T-0017](tickets/T-0017-windows-wasapi-output.md) Windows WASAPI output + cpal fallback | 8e16368 | build-1/2, review-1 (changes), review-2 (approve); incl. engine position fix |
| [T-0006](tickets/T-0006-signals-db-writer.md) signals.db & batched writer | 445df54 | build-1/2, review-1 (changes), review-2 (approve); 39.4 B/event |
| [T-0030](tickets/T-0030-engine-append-and-docs.md) Engine follow-ups | 975e812 | build-1, review-1 (approve) |
| [T-0028](tickets/T-0028-engine-pairing-fixes.md) Engine pairing fixes + property test | c0bfd2a | build-1, review-1 (approve); follow-ups T-0030 |
| [T-0009](tickets/T-0009-spike-time-stretch.md) Spike: time-stretch | 176c82e | spike-1 → ADR 0006, T-0026, T-0027 |

Proposals P-0001..P-0224 all have outcomes (see work/proposals/). Parked: P-0003/P-0033/P-0037/P-0044/P-0047/P-0065/P-0066/P-0076 (CI —
needs a remote), P-0016 (worktree EOL refresh), P-0027 (IAudioClock interpolation), P-0054 (rate crossfade), P-0057 (silence for corrupt packets), P-0058 (comment), P-0063 (provider watchdog), P-0078 (T-0014 nits), P-0083 (loopback gapless test), P-0094 (PlayStarted volume, E-04), P-0130 (textFaint/textMuted step — PO look pass), P-0135 (StreamInfo to the shell), P-0136 (pre-hear skips, Phase 7), P-0139 (stress in CI), P-0142 (library ArtworkSource contract), P-0149 (bind QCoreApplication::exit), P-0152 (writer commit notify), P-0162 (queue mirror), P-0165 (cargo-about), P-0167 (packaging in CI), P-0170 (icon scan quotes), P-0175 (capture key delivery), P-0176 (focus helper), P-0178 (no-artwork warnings), P-0179 (aspect query), P-0184 (image-stress relink), P-0185 (visible-rows paging), P-0187 (Settings-driven appearance), P-0188 (other-OS probes), P-0220 (sub-frame seeks), P-0222 (resampler priming), P-0192 (XSVG round-trip), P-0193 (installer manifest).
