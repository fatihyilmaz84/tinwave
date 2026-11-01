# Tinwave — Development Rules (read first)

Tinwave is a local-first music library manager, player and cross-device ecosystem.
Rust core, Qt 6 Quick/QML desktop UI (via CXX-Qt), SQLite. Desktop comes first; mobile later.

The product spec is [tinwave-specs.md](tinwave-specs.md). These rules refine it — they never override it.
When a rule here conflicts with the spec, the spec wins until it is updated (see "Changing decisions").

## The two big ideas

1. **Own your music.** Offline, no account, no cloud required, nothing silently changes user files.
2. **Know the listener better than any streaming service — on their own machine.**
   Tinwave captures every listening signal it can (volume moves, seeks, skips, speed changes,
   time, weather, device, activity…) and turns them into recommendations with human explanations.
   All of it stays on-device. Nothing is sent anywhere except an opt-in, coarse weather lookup.

## How work is organized

Tinwave is built by a small agent team. The process is in [work/README.md](work/README.md).
- **PO** (human): roadmap, decisions, epic acceptance.
- **Lead** (main session on Opus, `/lead` skill): plans, writes tickets, dispatches, reviews results, merges. Only writer of `work/`.
- **Builders** (`tw-builder`, Sonnet): one ticket each, in a worktree; suggest work as proposals in their handoff.
- **Reviewer** (`tw-reviewer`, Opus): read-only verdict on every branch before it reaches `main`.
- **Spikes** (`tw-spike`, Sonnet): time-boxed prototypes that end in a draft ADR.

If you are a subagent: your ticket is your contract, stay inside its `scope`, never edit `work/`, end with the
handoff format in your agent definition.

## Required reading before writing code

| Doc | When |
|---|---|
| [docs/rules/00-principles.md](docs/rules/00-principles.md) | Always |
| [docs/rules/architecture.md](docs/rules/architecture.md) | Before creating or touching a crate |
| [docs/rules/rust.md](docs/rules/rust.md) | Any Rust code |
| [docs/rules/qml-ui.md](docs/rules/qml-ui.md) | Any UI / bridge code |
| [docs/design/design-system.md](docs/design/design-system.md) | Any UI: tokens, native controls, `Tw*` components, platform rules, UI Definition of Done |
| [docs/rules/database.md](docs/rules/database.md) | Schema, queries, migrations |
| [docs/rules/storage-budget.md](docs/rules/storage-budget.md) | Anything that persists signals or derived data |
| [docs/rules/testing.md](docs/rules/testing.md) | Every change |
| [docs/rules/privacy-and-signals.md](docs/rules/privacy-and-signals.md) | Anything that records, reads or derives user data |
| [docs/guides/desktop-dev-setup.md](docs/guides/desktop-dev-setup.md) | Setting up a machine |
| [docs/guides/signal-catalog.md](docs/guides/signal-catalog.md) | The full list of signals we collect |
| [docs/guides/context-engine.md](docs/guides/context-engine.md) | Time / weather / system context providers |
| [docs/guides/audio-analysis.md](docs/guides/audio-analysis.md) | Per-track DSP features |
| [docs/guides/taste-engine.md](docs/guides/taste-engine.md) | Turning signals into taste |
| [docs/guides/smart-radio.md](docs/guides/smart-radio.md) | Recommendation sessions and features |
| [docs/guides/adding-a-signal.md](docs/guides/adding-a-signal.md) | Adding any new signal |
| [docs/adr/](docs/adr/) | Architecture decisions |

## Hard rules

- Implement in small, testable phases. Never generate the whole app in one pass.
- No hacks, placeholder implementations, silent fallbacks or permanent `TODO`s in a completed phase.
- No browser engine, no JS runtime, no resident ML model, no always-running background service.
- The UI never touches SQLite directly. It talks to Rust services through the bridge.
- **Native-first UI** ([design system](docs/design/design-system.md), ADR 0003): OS controls, chrome, menus and dialogs;
  custom drawing only through `Tw*` components; no raw colors/sizes in QML — tokens from `Theme.qml` only.
- Playback must never block on anything: not the DB, not analysis, not a context provider, not the network.
- **Every user interaction that changes playback or expresses preference MUST emit a signal event**
  (see [signal catalog](docs/guides/signal-catalog.md)). A playback feature without its signals is not done.
- **Never add a signal without** updating the signal catalog, the Signal Center UI list, and its privacy tier.
- Collected data must stay small: signal + taste data under ~150 MB after 5 years for a heavy listener.
  Count instead of logging high-frequency signals, coalesce gestures, quantize payloads, compact
  hot → warm → cold. The storage growth test must pass ([storage-budget.md](docs/rules/storage-budget.md)).
- Signals are instrumented from Phase 1. Smart Radio ships in Phase 7, but history cannot be backfilled.
- Never send user data off the device except what [privacy-and-signals.md](docs/rules/privacy-and-signals.md) explicitly allows.
- Never claim a feature is complete without passing its acceptance tests (spec §17).

## Commands

Build commands are fixed in Phase 0 once the Rust–Qt bridge is validated. Until then:

```
cargo build --workspace
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all --check
```

## Changing decisions

Write an ADR in `docs/adr/` (copy `0000-template.md`), update the spec section it affects,
and update the rule/guide that encoded the old decision — in the same change.
