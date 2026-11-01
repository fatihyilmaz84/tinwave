# ADR 0003 — Native-first desktop design system ("Quiet Native")

- **Status:** Accepted
- **Date:** 2026-10-29

## Context

Tinwave ships one Qt Quick/QML desktop app on Windows, macOS and Linux. The spec asks for a polished, calm,
music-focused UI (§14) that stays lightweight (§12) and avoids web-style runtimes (§2). Without a shared design
system, Sonnet builders would each invent colors, spacing and custom controls; the app would look like a web
app on every OS, and reviewers would have nothing objective to check against.

Options considered:
1. **Fully custom, brand-styled controls** (one look everywhere). Consistent, but fights each OS, costs a lot
   of QML, and every control needs its own accessibility and input work.
2. **Material / Universal Qt styles everywhere.** Cheap, but foreign on all three desktops.
3. **Native-first:** Qt Quick Controls native styles per OS for all standard controls, plus a small set of
   token-driven `Tw*` components for music-specific UI.

## Decision

Option 3.

- Standard controls, chrome, menus and dialogs come from the OS: `FluentWinUI3` (Windows), `macOS`, `Fusion`
  with the platform palette (Linux). Native menus (`Popup.Native`), native file dialogs, global menu bar on macOS,
  system font, system accent and system light/dark by default.
- Minimum Qt version **6.8 LTS** (needed for FluentWinUI3, native Qt Quick menus, `setColorScheme`, `QPalette::Accent`).
- All custom UI uses semantic tokens from a single `Theme.qml`; no raw values in QML.
- A fixed component catalog (`Tw*`), each component declaring the catalog signals it emits.
- OS media integration (SMTC, MPNowPlaying, MPRIS) and window effects (Mica, vibrancy) are part of the system,
  behind traits with `Unsupported` fallbacks.
- The system is documented in `docs/design/design-system.md`; the PO's design canvas illustrates it.

## Consequences

- Less custom QML, better accessibility and input behavior for free, lower memory than heavy custom styling.
- The app looks slightly different per OS by design; screenshots and reviews must name the platform.
- Qt 6.8 is a hard floor; native style availability must be validated per OS (spike T-0011).
- Reviewers get an objective checklist (design-system.md §8). New components or tokens need a design ticket.
- Fusion on Linux is the least "native" option; revisit if Qt gains a better Linux-native Quick style.

## Spec / docs updated

- `tinwave-specs.md` §2 (Qt version floor) and §14 (design system).
- `docs/design/design-system.md` (new), `docs/rules/qml-ui.md`, `docs/rules/00-principles.md`, `CLAUDE.md`.
- Process: `work/README.md`, `work/templates/ticket.md`, `.claude/agents/tw-builder.md`, `tw-reviewer.md`,
  `tw-spike.md`, `.claude/skills/lead/SKILL.md`, `.claude/skills/ticket/SKILL.md`.
