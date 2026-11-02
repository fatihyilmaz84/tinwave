# Board

_Maintained by the lead. Last updated: 2026-11-02._

## Needs the PO
- **Check your system volume.** T-0008 left master volume at 2 % unmuted; T-0003 later found the default output
  muted at 6 %. Set it the way you want before any listening test.
- Install the Qt **MSVC 2022 64-bit** kit (Qt Maintenance Tool → Qt 6.8 LTS). Only `mingw_64` is installed; it cannot
  link with the MSVC Rust toolchain. Blocks T-0002 and T-0011.
- Runbooks, when convenient (they gate acceptance, not current work):
  [T-0008 PO-RUNBOOK](../spikes/T-0008-win-volume/PO-RUNBOOK.md) R1–R3, R5–R9 and
  [T-0003 PO-RUNBOOK](../spikes/T-0003-audio-backend/PO-RUNBOOK.md) sections 1, 2, 4c, 5.
- **Listening pass (ADR 0006):** run `cargo run --release -- quality wav` in `spikes/T-0009-time-stretch/`
  and listen to `out/*.wav` — are the softer hi-hats at 0.75× and the ~15 ms dip per rate step OK for v1?
  Only a "no" would bring up Rubber Band (GPL/commercial licence decision).
- Optional cleanup: the T-0009 spike extracted `libclang.dll` (PyPI `libclang` 18.1.1 wheel, LLVM) into temp
  folders only (`%TEMP%\claude\libclang_pkg` and the session scratchpad); nothing installed or committed.
- For T-0020: install ffmpeg (or allow the agent to) and point to a folder with a few real M4A albums and other formats.
- Delete leftovers: `_move-into-.claude/`, `Claude outputs/`, and
  `D:\dev\Tinwave-worktrees\T-0008-spike-windows-volume-device\` (all merged/installed).

## In progress
| Ticket | Owner | Branch / worktree |
|---|---|---|
| [T-0028](tickets/T-0028-engine-pairing-fixes.md) Engine pairing fixes + property test (P0, D-0005) | tw-builder r1 | `t/T-0028-engine-pairing-fixes` · `D:\dev\Tinwave-worktrees\T-0028-engine-pairing-fixes` |
| [T-0014](tickets/T-0014-windows-volume-device-provider.md) Windows output-device + system-volume providers | tw-builder r1 | `t/T-0014-windows-volume-device-provider` · `D:\dev\Tinwave-worktrees\T-0014-windows-volume-device-provider` |
| [T-0006](tickets/T-0006-signals-db-writer.md) signals.db & batched writer | tw-builder r1 | `t/T-0006-signals-db-writer` · `D:\dev\Tinwave-worktrees\T-0006-signals-db-writer` |

Worktrees are made by the lead with `git worktree add` under `D:\dev\Tinwave-worktrees\` (the Agent tool's built-in
worktree isolation fails on this machine: the `^{commit}` ref loses its braces).

## In review
_none_

## Ready (dispatch order)
| Ticket | Type | Size | Scope | Note |
|---|---|---|---|---|
| [T-0002](tickets/T-0002-spike-cxx-qt.md) Spike: CXX-Qt | spike | M | spikes/T-0002 | needs Qt MSVC kit |
| [T-0011](tickets/T-0011-spike-native-qt-styles.md) Spike: native Qt styles & Theme | spike | M | spikes/T-0011 | needs Qt MSVC kit |

Max 3 agents in flight, reviewers included. Root `Cargo.toml` [workspace.dependencies] and `Cargo.lock` don't count as
overlap (work/README.md); the branch merging second merges main first.

## Blocked
| Ticket | Waiting for |
|---|---|
| [T-0020](tickets/T-0020-spike-format-corpus.md) Spike: real-world format corpus | PO: ffmpeg + real files |

## Backlog
| Ticket | Depends on |
|---|---|
| [T-0007](tickets/T-0007-storage-growth-test.md) Growth test v1 | T-0006 |
| [T-0021](tickets/T-0021-persist-signal-settings.md) Persist Signal Center toggles (E-01) | T-0006 |
| [T-0024](tickets/T-0024-playback-controller.md) Playback controller: poll() + PlaybackEvents → signals (E-01, P0) | T-0006, T-0028 |
| [T-0017](tickets/T-0017-windows-wasapi-output.md) Windows WASAPI output (E-01) | T-0028 (same crate) |
| [T-0019](tickets/T-0019-resampler-and-dither.md) Resampler + dither (E-01) | T-0028 (same crate) |
| [T-0023](tickets/T-0023-live-queue-edits.md) Live queue edits (E-01) | T-0028 (same crate) |
| [T-0025](tickets/T-0025-flush-fade.md) Fade on seek/skip flush (E-01, P2) | T-0028 (same crate) |
| [T-0026](tickets/T-0026-signalsmith-wrapper.md) Vendored Signalsmith wrapper (E-01) | T-0028 (same crate) |
| [T-0027](tickets/T-0027-rate-control-engine.md) Rate control in the engine (E-01) | T-0026 |
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
| [T-0009](tickets/T-0009-spike-time-stretch.md) Spike: time-stretch | 176c82e | spike-1 → ADR 0006, T-0026, T-0027 |

Proposals P-0001..P-0059 all have outcomes (see work/proposals/). Parked: P-0003/P-0033/P-0037/P-0044/P-0047 (CI —
needs a remote), P-0016 (worktree EOL refresh), P-0027 (IAudioClock interpolation), P-0054 (rate crossfade), P-0057 (silence for corrupt packets), P-0058 (comment).
