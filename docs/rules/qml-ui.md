# Qt / QML & bridge rules

## Structure

- QML is presentation only. Logic lives in Rust view models exposed through CXX-Qt `QObject`s.
- One view model per screen area (Library, Album, Artist, Playlists, NowPlaying, SmartRadio, SignalCenter,
  Insights, Sync, Servers, Import, Settings).
- Lists use `QAbstractListModel` subclasses backed by paginated Rust queries. Use `ListView`/`GridView`
  with `reuseItems: true`. Never expose a full library as a JS array.
- Artwork via a custom `QQuickAsyncImageProvider` with a bounded LRU cache and size-specific thumbnails.
  Image providers follow ADR 0008's lifetime addendum: responses live only on Qt's reader thread; workers reach them only
  through a queued `done` signal; exactly one `finished()` per response (`cancel()` only sets a flag); a bounded pool with a
  closing flag; every provider is quiesced (closing + `waitForDone()`) before the QML engine is dropped, enforced by the
  app's RAII UI owner — never rely on destructors alone.
- `#[allow(unsafe_code)]` for a bridge goes on its `mod name;` declaration, not on the `#[cxx_qt::bridge]` module
  (CXX-Qt rejects other attributes there; ADR 0008).
- Controls with a text mnemonic (`ItemDelegate`, `Button`, `MenuItem`…) treat `&` as a shortcut marker: write a literal
  ampersand as `&&` ("Sources && devices").
- Developer-only switches (e.g. `TINWAVE_MAIN_QML`) are read under `#[cfg(debug_assertions)]` only; a release build never
  loads UI from a path or URL taken from the environment.

## Threading

- The Qt thread never waits on Rust IO. Rust work posts results back with `qt_thread().queue(...)`.
- Long operations show progress and are cancellable from the UI.

## UI-originated signals

The UI is a major signal source (browse dwell, hover, scroll impressions, view changes). Rules:

- UI emits signals through a single `SignalEmitter` QObject (`emitter.browse(...)`, `emitter.hover(...)`),
  never by calling the DB.
- Debounce in Rust, not QML: hover ≥ 800 ms, dwell measured on page leave, scroll impressions sampled
  when the list settles (≥ 500 ms still).
- Every playback control (buttons, sliders, keyboard shortcuts, tray menu, media keys, mini-player) must pass
  its **input source** (`Mouse`, `Keyboard`, `MediaKey`, `Tray`, `MiniPlayer`, `Remote`) — the source is part of the signal.
- Volume and speed sliders emit one event per *gesture* (press → release, or wheel burst ended after 600 ms),
  with `from`, `to` and the playback position at gesture start. Never one event per pixel.

## Design (see [design-system.md](../design/design-system.md), ADR 0003)

- **Native first.** Standard controls, menus, dialogs and window chrome come from the OS via Qt Quick Controls
  native styles (FluentWinUI3 / macOS / Fusion + platform palette), selected at startup. Never restyle a native
  control's background or content item; set size, layout and `palette` roles from tokens only.
- **Tokens only.** No raw colors, font sizes, spacing, radii or durations in QML outside `qml/Theme/Theme.qml`.
  System font, OS accent and OS light/dark by default, with user overrides.
- **Custom UI = `Tw*` components only** (`qml/Tw/`), from the catalog in design-system.md §5. A new custom
  component or token needs a design ticket that updates design-system.md first.
- Calm, music-first: neutral chrome, artwork carries the color; light and dark themes.
- Always show the playback source badge (`TwSourceBadge`: Local / Server name / Downloaded).
- Every recommendation shows its explanation (`TwWhyChip`, "Why this?") on hover, long-press and Enter.
- OS media integration (SMTC / MPNowPlaying / MPRIS), tray/dock menus and the mini-player call the same
  view-model methods as the main window, with their own `InputSource`.
- Accessibility: keyboard navigation for everything, accessible names, visible focus, ≥ 4.5:1 text contrast,
  OS reduced motion and high contrast respected.
- UI work follows the design Definition of Done in design-system.md §8.

## Signal Center screen (required, Phase 1)

- Category list with tier badge, on/off toggle, event count, last event time.
- Live feed of the last 100 events (human-readable).
- Storage panel: size of each store vs its budget, hot/warm/cold window lengths (editable), last compaction.
- Export (compressed JSON Lines) and wipe per category / per tier / everything, with confirmation.
- Shows exactly what the weather request sends (rounded coordinates, provider URL).
