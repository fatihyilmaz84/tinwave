# Tinwave — Product & Technical Specification

**Product:** Tinwave  
**Category:** Local-first music library manager, player, organizer, and cross-device music ecosystem  
**Platforms:** Windows, macOS, Linux, iOS, Android  
**Architecture:** Rust core, Qt 6/QML desktop UI, native SwiftUI and Jetpack Compose mobile apps  
**Primary principle:** Your music belongs to you. Tinwave works offline, does not require an account or cloud subscription, and integrates with self-hosted music servers.

---

## 1. Product vision

Tinwave is a premium, cross-platform music ecosystem for people who own their music collections and want complete control over how they organize, play, discover, and synchronize music.

It combines the capabilities of a desktop music library manager, an offline mobile music player, a personal music recommendation system, and a bridge to existing self-hosted media servers.

Tinwave must be useful without an internet connection, cloud account, or subscription.

### Core product pillars

1. **Own your music:** Manage local audio files and metadata without vendor lock-in.
2. **Listen everywhere:** Synchronize selected music to phones over local Wi-Fi and play it offline.
3. **Connect your servers:** Integrate with Jellyfin, Plex, and other self-hosted music services.
4. **Keep your library organized:** Correct tags, artwork, filenames, and folder structures safely.
5. **Discover your next favorite:** Build personal radio stations and playlists using local listening history.
6. **Keep it lightweight:** Prioritize low memory use, fast startup, and responsive browsing.
7. **Protect privacy:** Store listening history and taste profiles locally by default.

## 2. Supported platforms and technology

### Desktop

- Windows
- macOS
- Linux

**Technology**
- Rust for application logic, library indexing, synchronization, file operations, metadata, and audio-related services.
- Qt 6 Quick/QML for the desktop user interface (minimum Qt 6.8 LTS, native Qt Quick Controls styles per OS — ADR 0003).
- Evaluate CXX-Qt as the Rust–Qt bridge in a Phase 0 proof of concept.
- SQLite for the local database.
- SQLite FTS5 for fast text search.
- A Rust-compatible audio backend selected after format, device, gapless playback, and cross-platform testing.
- FFmpeg for optional conversion, audio analysis, and format compatibility where appropriate.

The desktop application must not embed a browser-based UI runtime. Avoid unnecessary JavaScript engines, heavyweight frameworks, and always-running background services.

### Mobile

- iOS: SwiftUI.
- Android: Kotlin and Jetpack Compose.
- Rust shared libraries for selected portable domain logic, protocol handling, metadata normalization, and synchronization where this is beneficial.

Mobile interfaces must use platform-native lifecycle, storage, notification, audio, and background-transfer APIs.

### Architectural rule

Keep the Rust domain layer independent of the UI framework, operating system, database implementation, and external server integrations wherever practical.

Use clear interfaces between the library core, local storage, playback, sync engine, server connectors, and UI.

Do not force desktop, iOS, and Android to share UI code merely for the sake of code reuse.

## 3. Core product modes

Tinwave supports three complementary ways of accessing music.

### A. Tinwave-managed local library

Tinwave indexes and manages music files stored on the user's computer or attached storage.

Capabilities include:
- Import and index existing folders.
- Browse artists, albums, tracks, genres, and releases.
- Edit metadata and artwork.
- Safely organize files into a preferred directory structure.
- Build playlists and smart playlists.
- Play music and maintain listening history.
- Synchronize selected music to mobile devices.
- Rip CDs into a managed music collection.

The user retains ownership of the underlying files and can access them without Tinwave.

### B. Connected self-hosted servers

Tinwave connects to existing media servers and presents their music alongside local music.

Initial integration priorities:

1. Jellyfin
2. Plex Media Server
3. Navidrome
4. Emby
5. Subsonic-compatible servers, including compatible OpenSubsonic implementations where feasible

Additional servers can be supported through a connector interface rather than custom logic throughout the application.

Connected-server support must be optional. Tinwave must remain fully functional when no server exists.

### C. Offline mobile library

The mobile apps maintain their own local database, cached metadata, playlists, and downloaded audio.

A phone must continue to play downloaded music and access its local playlists, favorites, and listening history when:
- The desktop is shut down.
- The media server is unavailable.
- The internet is unavailable.
- The user is traveling.

## 4. Self-hosted media server integrations

### 4.1 Integration goals

Tinwave should complement existing self-hosted music setups rather than replace them.

Users must be able to connect an existing server, browse its music, play tracks, and optionally save tracks for offline listening.

The first release of each connector should focus on reliable music access, not on managing the entire server.

### 4.2 Jellyfin integration

Support the documented Jellyfin server API for:
- Server connection and authentication.
- Music libraries, artists, albums, and tracks.
- Search and browsing.
- Track streaming and playback.
- Album artwork and available metadata.
- Playlists and favorites where supported by the API.
- Playback reporting where supported.
- Downloading tracks for offline playback when permitted and supported.

The connector must handle server versions, pagination, authentication failures, unavailable media, and interrupted streams.

Reference documentation: https://jellyfin.org/docs/

### 4.3 Plex Media Server integration

Support Plex Media Server's documented API for:
- Server discovery or manual server URL entry.
- Account/server authentication using supported methods.
- Music library browsing and search.
- Artist, album, track, and artwork retrieval.
- Audio streaming and playback.
- Playlist and rating/favorite operations where supported.
- Offline downloads when supported by the API and the server's permissions.

Plex authentication and authorization must use supported mechanisms. Do not scrape the Plex web interface, embed private credentials in the app, or assume every server endpoint is publicly documented.

The connector must account for Plex server reachability, remote access configuration, token expiry, and API compatibility.

Reference documentation: https://developer.plex.tv/

### 4.4 Navidrome, Emby, and Subsonic-compatible servers

Implement these as separate connectors behind a shared server interface.

For Navidrome and compatible servers, evaluate Subsonic/OpenSubsonic APIs and server-specific capabilities. Do not assume that every server supports identical playlist, favorites, scrobbling, transcoding, or download behavior.

Emby support should use its documented and supported interfaces.

Each connector must declare its actual capabilities so the UI does not display unsupported operations.

### 4.5 Shared connector interface

Define a Rust interface conceptually equivalent to:

- `connect()`
- `authenticate()`
- `get_server_info()`
- `get_capabilities()`
- `search()`
- `list_artists()`
- `list_albums()`
- `list_tracks()`
- `get_track_stream()`
- `get_artwork()`
- `get_playlists()`
- `update_favorite()` where supported
- `report_playback()` where supported
- `download_track()` where supported
- `disconnect()`

The exact trait signatures should be decided during implementation.

Every connector must use pagination, bounded concurrency, caching, cancellation, and clear error reporting. A failed server must not freeze the UI or interrupt playback from another source.

### 4.6 Server connection and security

Users can add a server by entering its address or selecting it through a supported discovery mechanism.

Requirements:
- Support local-network servers and appropriately configured remote servers.
- Use encrypted transport when available and validate certificates correctly.
- Never disable certificate validation silently.
- Store credentials and tokens in the operating system's secure credential store.
- Provide a disconnect and credential-revocation workflow.
- Never expose server credentials in logs, URLs displayed to users, crash reports, or sync payloads.
- Explain when a server is reachable only on the home network.
- Do not automatically expose a server to the public internet.
- Never require a Tinwave account to connect a local server.

Local discovery is a convenience, not authentication. Remote access remains dependent on the user's server and network configuration.

### 4.7 Source identity and duplicate handling

The same album or recording may exist in a local library and on multiple servers.

Tinwave must distinguish:
- A recording as a musical entity.
- A specific local media file.
- A server-specific media item.
- A playable source or stream.
- A downloaded offline copy.

Use stable internal identities and source-specific IDs. Do not merge items solely because their filenames or titles match.

Where a recording is matched across sources, Tinwave may present one logical recording with multiple playback sources. The user must be able to see where the playable version comes from.

### 4.8 Server and local library boundaries

Connecting a server must not automatically copy, move, tag, rename, or delete files on that server.

Default server access should be read-only except for explicit user actions that the connector supports.

Local library organization and server library management must be separate features. Server-side file changes should be added only if they can be performed safely, with clear confirmation and a reliable understanding of server permissions.

Tinwave must not claim that server data is fully synchronized when the server API exposes only partial capabilities.

### 4.9 Offline use of server music

Users can optionally save supported server tracks to the phone for offline playback.

The download workflow must:
- Show available file size where known.
- Check available storage.
- Resume interrupted transfers when possible.
- Verify downloaded file integrity.
- Keep temporary and completed downloads separate.
- Store source and license/access information where relevant.
- Respect server authorization and content access restrictions.
- Let users remove a mobile copy without deleting the server original.

A server connection is not required to play already-downloaded files.

### 4.10 Playback and listening history

Tinwave can maintain a unified listening history across local files and connected servers.

Each event should record its source, recording identity where known, timestamp, device, and playback details.

Users control whether listening events are:
- Stored only on the current device.
- Shared between Tinwave devices.
- Reported to a connected server, where supported.
- Included in Tinwave's local Smart Radio calculations.

Reporting playback to Plex, Jellyfin, or another server must be configurable. Never assume that a server's listening history and Tinwave's own history are automatically equivalent.

### 4.11 Integration roadmap

- **Early architecture:** Define connector interfaces and source identity.
- **Desktop library phase:** Build local playback and browsing first.
- **Initial server integration phase:** Implement Jellyfin as the first connector and validate real-world music browsing and streaming.
- **Second connector:** Add Plex using its documented API and authentication flow.
- **Expansion:** Add Navidrome, Emby, and compatible Subsonic servers based on demand and API feasibility.
- **Mobile:** Reuse connector concepts where practical, while implementing platform-specific networking, secure storage, playback, and downloads.

Do not postpone local library functionality until all server connectors are implemented.

## 5. Data model

Separate musical identity from physical files and external server items.

Core entities:
- Artist
- ReleaseGroup
- Release
- Recording
- ReleaseTrack
- MediaFile
- FileLocation
- ArtworkAsset
- Playlist
- PlaylistItem
- ListeningEvent
- Device
- ImportJob
- FileOperation
- RipJob
- SyncOperation
- ServerConnection
- ServerItem
- PlaybackSource
- SmartPlaylistRule
- TasteProfile

Store external provider IDs and provenance separately from user-edited metadata.

Use migrations, transactions, indexes, and stable IDs. Keep large artwork, audio data, and other binary files out of ordinary database rows unless there is a compelling reason.

## 6. Local library management

### Import and scanning

- Import one or more folders.
- Support incremental rescanning.
- Detect renamed or moved files where practical.
- Avoid rehashing entire files when reliable cached fingerprints and file metadata are sufficient.
- Offer a full verification scan when requested.
- Report unsupported, unreadable, and damaged files.
- Keep scans cancellable and recoverable.

### Metadata

Read embedded tags and support common formats, including FLAC, MP3, M4A/AAC, ALAC, and WAV where metadata is available.

Support:
- Title, artist, album artist, album, track/disc numbers, year, genre, and composer.
- Embedded and external artwork.
- Multiple artists and collaborations.
- Compilation albums.
- Multi-disc releases.
- User ratings and favorites.
- Optional lyrics and other metadata where available.

Metadata enrichment should use replaceable provider adapters, including MusicBrainz and compatible acoustic fingerprinting services.

External services are optional. Respect provider rate limits, API conditions, and commercial-use requirements.

### Safe organization

Provide a preview before renaming, moving, or reorganizing files.

Requirements:
- Dry-run mode.
- Operation journal and recovery.
- Collision detection.
- No silent overwrites.
- No automatic deletion of suspected duplicates.
- Tag backups or reversible changes where feasible.
- Clear reporting of files that could not be processed.
- User-configurable folder and filename templates.

## 7. Audio playback

Build a shared playback service independent of the UI.

Required capabilities:
- Play, pause, seek, next, previous, queue, repeat, and shuffle.
- Gapless playback where supported.
- Resume after restart.
- Queue persistence.
- Output-device changes where supported.
- Audio focus and interruption handling on mobile.
- Playback from local files, mobile downloads, and connected servers.
- Clear error recovery when a file or stream becomes unavailable.

Evaluate playback backends through a real-world format and performance test rather than choosing one solely on theoretical feature coverage.

Use FFmpeg for optional conversion or analysis where appropriate, not as a justification for loading large processing libraries unnecessarily in every process.

## 8. Local Wi-Fi synchronization

Local Wi-Fi sync is a core feature and must not depend on cloud services.

### Roles

The desktop can act as the primary library and sync host, while each phone retains its own local database and offline files.

The phone must remain useful when the desktop is offline.

### Pairing and discovery

- Discover the desktop on the local network using Bonjour/mDNS or another suitable cross-platform mechanism.
- Provide manual server address entry as a fallback.
- Pair through a QR code or short-lived pairing code.
- Require explicit user confirmation on both devices.
- Use authenticated, encrypted transport.
- Allow devices to be named, reviewed, disconnected, and revoked.

Discovery alone must never grant access.

### Sync capabilities

- Select playlists, artists, albums, tracks, and favorites for download.
- Show storage use and transfer progress.
- Resume interrupted transfers.
- Verify checksums.
- Use temporary files until transfers are verified and finalized.
- Support cancellation and bounded transfer concurrency.
- Handle Wi-Fi loss, desktop sleep, phone suspension, and application restart.
- Avoid repeated full-library scans and full-file transfers when no content has changed.
- Optionally transcode to a smaller mobile format while preserving the desktop original.

### Sync channels

Make these independently configurable:
1. Audio files.
2. Library metadata.
3. Playlists and playlist ordering.
4. Favorites and ratings.
5. Listening events.
6. Preferences.

Device-specific settings remain local by default.

### Conflict handling

- Listening events are append-only and deduplicated by event ID.
- Metadata edits retain provenance and use explicit conflict rules.
- Playlist ordering conflicts are resolved deterministically and surfaced when needed.
- Deletions use tombstones so offline devices can catch up.
- Removing a downloaded file from a phone does not delete the desktop original.
- Sync must never silently overwrite user changes.

## 9. Native mobile applications

Both mobile apps must provide:
- Offline music playback.
- Local playlists, favorites, and listening history.
- Browsing and searching downloaded music.
- Background playback and lock-screen controls.
- Queue persistence.
- Selective Wi-Fi sync.
- Server connections and streaming.
- Offline downloads from supported servers.
- Clear storage management.
- Resume of interrupted sync operations.
- Deferred changes that can synchronize later.

Respect iOS and Android background-execution rules. Transfers should checkpoint progress and resume when the operating system permits; do not promise unrestricted background syncing.

## 10. Smart Radio and recommendations

Smart Radio creates personalized listening sessions from the user's library and permitted server sources.

Initial recommendations should be lightweight and explainable, using:
- Favorites and ratings.
- Play counts and recency.
- Skips and early exits.
- Replays and completion.
- Artist and album affinity.
- Genre and metadata similarity.
- Diversity and novelty.
- Context such as recently played tracks.
- Repetition penalties.

Users can choose a starting artist, album, genre, track, or mood-like collection.

Provide options to favor familiarity, discovery, diversity, or a balance of these.

Listening data and taste profiles stay local by default. Do not require a cloud AI service or a permanently loaded machine-learning model.

Advanced recommendation models may be optional future enhancements.

### 10.1 Listening signals and context (added 2026-10-29, ADR 0001)

Recommendations also use fine-grained on-device signals and listening context, captured from Phase 1:
- Volume and system-volume changes, mapped to the song section where they happen.
- Playback-speed changes, seeks, replayed segments, skip position and input source.
- Queue edits, browsing attention, search (including searches with no result).
- Output device, time of day, weekday, holidays, season, daylight, power and idle state.
- Optional (opt-in): foreground-application category, network place, calendar busy state, and weather.

All signals stay on the device except an opt-in weather request with rounded coordinates.
Users can view, export, and wipe every signal category in a Signal Center.

Signal data must stay small (ADR 0002): high-frequency interactions are stored as counters, raw events are
compacted after 90 days into per-play records and later monthly rollups, and signal plus taste data must
remain under roughly 150 MB after five years for a heavy listener. A storage growth test enforces this.

Details: `docs/guides/signal-catalog.md`, `docs/guides/taste-engine.md`, `docs/guides/smart-radio.md`,
`docs/rules/privacy-and-signals.md`.

## 11. CD ripping

Implement ripping after the core library, playback, and synchronization workflows are stable.

Requirements:
- Detect supported optical drives.
- Read track layout and disc metadata.
- Retrieve metadata from optional online sources.
- Rip to FLAC as an archival option.
- Offer portable-format conversion where appropriate.
- Support secure rereads and clear error reporting.
- Use AccurateRip or a compatible verification mechanism where available and legally/technically feasible.
- Preserve track order, disc number, and album metadata.
- Support retries, cancellation, and recovery after failure.
- Allow users to review metadata and filenames before committing the rip.

Test actual optical-drive compatibility on Windows, macOS, and Linux. Do not assume that virtual drives or every USB optical drive behave identically.

## 12. Lightweight performance requirements

Performance must be measured on representative hardware across all desktop platforms.

Initial engineering targets, subject to prototype validation:
- Idle desktop memory: aim for approximately 50–100 MB.
- Normal browsing and playback: aim for approximately 80–160 MB.
- Scanning/import: aim for approximately 150–300 MB.

These are targets, not guarantees, and must include relevant Qt and helper-process memory.

Required techniques:
- No browser engine in the desktop UI.
- Lazy loading of artwork and metadata.
- Virtualized lists and grids.
- Paginated database queries.
- Bounded caches with configurable limits.
- Bounded worker queues.
- No unbounded in-memory collection of the entire library.
- Avoid redundant database copies and expensive serialization.
- No permanently loaded ML models by default.
- Pause or reduce background work while idle or on battery where appropriate.
- Avoid unnecessary helper processes and resident services.

Measure cold startup, warm startup, search latency, scrolling, scan throughput, audio start time, memory peaks, and idle CPU.

## 13. Optional remote access and cloud sync

Cloud sync is optional and must not be required for core functionality.

Local Wi-Fi sync should be implemented first. The same versioned protocol may later support a remote sync service.

A future hosted service could use:
- A Rust sync service.
- PostgreSQL for account/device and synchronization metadata.
- Object or chunk storage for audio where applicable.
- Encryption in transit and at rest.
- Optional end-to-end encryption where the service must not have access to audio or user data.

The architecture must not assume that every user has an account or uses hosted infrastructure. Self-hosting the sync service may be supported later.

## 14. User experience and product boundaries

The interface should feel polished, calm, responsive, and music-focused.

Primary desktop areas:
- Library.
- Artists and albums.
- Playlists.
- Smart Radio.
- Downloads and sync.
- Connected servers.
- Import and organization.
- CD ripping.
- Settings and diagnostics.

The mobile app should prioritize Now Playing, library browsing, offline downloads, playlists, search, and server selection.

Always distinguish local files, server-streamed tracks, and downloaded server copies. Show the source clearly without making users understand the internal architecture.

### 14.1 Desktop design system (added 2026-10-29, ADR 0003)

The desktop UI follows a native-first design system ("Quiet Native"): the operating system provides window
chrome, menus, dialogs and standard controls through Qt Quick Controls native styles (FluentWinUI3 on Windows,
macOS style on macOS, Fusion with the platform palette on Linux), using the system font, accent color and
light/dark setting. Tinwave draws only music-specific components, from a single token file. Album artwork
carries the color; chrome stays neutral. OS media controls (SMTC, Now Playing, MPRIS) are integrated.

Details: `docs/design/design-system.md`.

Do not bundle unnecessary features merely to match competitors. Keep advanced controls available without overwhelming everyday use.

## 15. Security, privacy, licensing, and commercial readiness

- No mandatory account or cloud service.
- Local listening history by default.
- Secure storage of server credentials and pairing secrets.
- Encrypted network transport.
- Explicit permissions and revocation.
- No telemetry by default; any optional diagnostics must be clearly disclosed and configurable.
- Avoid sending library filenames, listening history, or metadata to external services without user consent.
- Maintain a dependency inventory and Software Bill of Materials.
- Review commercial-use terms and redistribution obligations for Qt, audio libraries, codecs, metadata providers, and server APIs.
- Treat each server integration's API and authentication rules as a separate compliance requirement.
- Provide clear import/export and backup paths for user data.
- Avoid vendor lock-in through documented data export and ordinary audio files.

## 16. Development roadmap

**Build order (PO decisions D-0001, D-0002, 2026-11-01).** The phases below describe *what* each stage
contains; they are built in this order: Phase 0 → 1 → 2 → context engine and audio analysis → the
Smart Radio and statistics parts of Phase 7 → 3 → 6 → 4–5 → CD ripping (rest of Phase 7) → 8 → 9.
Desktop work is proven and measured on Windows first; macOS and Linux providers and measurements follow
before each phase's acceptance, behind traits that report `Unsupported` where an OS is not yet covered.

### Phase 0 — Feasibility and architecture
- Validate Rust + Qt Quick/QML and the chosen Rust–Qt bridge.
- Build a small playback prototype.
- Measure memory and startup time on each desktop OS.
- Validate SQLite schema and migration approach.
- Test mobile Rust library integration.
- Prototype a local Wi-Fi file transfer with checksum verification.
- Prototype Jellyfin connection, music browsing, and playback.
- Document architecture decisions and dependency licenses.

### Phase 1 — Desktop library foundation
- Local database and scanning.
- Basic artist, album, and track browsing.
- Search.
- Local playback and queue.
- Favorites and playlists.
- Artwork loading and caching.
- Signal bus, signal event store, T0/T1 context providers, and Signal Center (ADR 0001).

### Phase 2 — Metadata and safe organization
- Tag editing.
- Metadata provider adapters.
- Import review.
- Folder and filename templates.
- Preview, journal, and recovery for file operations.

### Phase 3 — Sync protocol and desktop host
- Device identity and pairing.
- Versioned protocol.
- Incremental metadata sync.
- Chunked/resumable file transfer.
- Checksum verification.
- Device management and revocation.

### Phase 4 — Native mobile apps
- Offline playback.
- Local database.
- Playlists and favorites.
- Mobile library browsing.
- Secure storage.
- Background playback and lifecycle handling.

### Phase 5 — Local Wi-Fi synchronization
- Desktop discovery and manual connection.
- QR/short-code pairing.
- Selective downloads.
- Progress, cancellation, and resume.
- Storage limits and optional mobile transcoding.
- Conflict handling and offline recovery.

### Phase 6 — Self-hosted server integrations
- Jellyfin connector.
- Plex connector.
- Source-aware playback and library browsing.
- Offline downloads where supported.
- Navidrome, Emby, and Subsonic-compatible connector expansion.
- Server capability reporting and credential management.

### Phase 7 — CD ripping and Smart Radio
- CD detection, ripping, verification, and recovery.
- Local listening statistics.
- Rule-based recommendations.
- Smart playlists and radio sessions.

### Phase 8 — Optional remote sync
- Reuse the local sync protocol.
- Authentication and device management if hosted accounts are introduced.
- Remote security review.
- Optional self-hosted deployment.

### Phase 9 — Commercial release
- Installers and updates for all desktop platforms.
- App Store and Google Play readiness.
- Accessibility, localization, crash recovery, and performance testing.
- Dependency and trademark/license review.
- Documentation, backup/export, and support workflows.

## 17. Acceptance tests

Tinwave is not ready for release until it can demonstrate:

- A large local library can be scanned and browsed without loading every item into memory.
- Search and playback remain responsive during background scanning.
- File organization can be previewed and recovered safely.
- A phone can pair with a desktop over Wi-Fi without internet access.
- A transfer can be interrupted, resumed, verified, and played offline.
- Revoking a device prevents future sync access.
- Playlist and listening-history changes behave predictably across offline devices.
- A Jellyfin server can be connected, browsed, streamed from, and disconnected without affecting local playback.
- A Plex server can be connected through supported authentication and API flows.
- Unsupported server capabilities are handled gracefully.
- Removing an offline download never deletes the server's original file.
- The application works without a Tinwave account or cloud subscription.
- Memory, CPU, startup, and playback performance are measured on all target desktop platforms.
- Third-party licenses and server API requirements are reviewed before commercial distribution.

## 18. Implementation rules for AI-assisted development

- Implement in small, testable phases.
- Do not generate the entire application in one pass.
- Write and maintain architecture decisions, setup instructions, and developer documentation.
- Keep code modular and avoid duplicate implementations.
- Do not introduce hacks, placeholder implementations, silent fallbacks, or permanent TODOs in completed phases.
- Add tests for domain logic, sync behavior, file operations, and connector capabilities.
- Prototype high-risk integrations before committing to a full implementation.
- Use real server instances and real devices for integration tests.
- Never claim a feature is complete without passing its acceptance tests.
- Update this specification when a technical decision changes, and document the reason for the change.

**Final product principle:** Tinwave is a local-first music ecosystem that works with music stored on the user's own devices and integrates with existing self-hosted servers. It must remain useful offline, respect user ownership, and avoid forcing cloud services or subscriptions on users.
