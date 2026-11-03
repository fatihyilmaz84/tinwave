//! The synthetic heavy listener, driven through the real pipeline.
//!
//! Everything the app would do goes through the same objects as in production: events are built
//! as real `SignalPayload`s and handed to `SignalBus::emit_with`; volume, system-volume, rate and
//! seek gestures are fed tick by tick to two `GestureCoalescer`s (the UI one and the OS-provider
//! one, each with its own bus clone); impressions, hovers and page dwells go to `UiCounters`; the
//! `SignalWriter` thread drains the bus into `signals.db` through `SignalStore`. Time is a
//! `ManualClock` and every random choice comes from a `SeededRng`, so a seed fixes the data.
//!
//! # Determinism
//!
//! The writer is a real thread, so the harness keeps it in lock step: one *tick* is one play.
//! Within a tick the UI counters are emitted first, then the play's events. The writer flushes at
//! every event that ends a play (`tinwave_db::ends_a_play`: `Completed`, `Skipped`, `Paused`,
//! `SessionEnded`), so the harness waits at a barrier after every such event until the writer
//! has committed everything accepted so far, before it emits anything else, touches the clock or
//! the counters. The last event of a tick always ends a play. Hence every transaction holds the
//! same events (those since the previous barrier, plus the counters the tick emitted first), in
//! the same order, with the same dictionary ids, whatever the thread scheduling. A `Paused` in
//! the middle of a play is the case that matters: without its barrier the writer would commit as
//! soon as it pulled `Paused`, and how many of the play's later events it had pulled by then
//! would depend on timing, and with it which dictionary row a new recording or counter entity
//! gets. `GrowthConfig::preempt_after_pause` stalls the producer after each `Paused` to prove it.
//!
//! # Plugging in compaction later
//!
//! [`run`] calls `on_day_end` after every simulated day, at a barrier (the writer is idle). A 5
//! year run with compaction runs the compaction pass from that hook, on its own connection.

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use rusqlite::{params, Connection};
use tinwave_core::{
    DeviceId, ManualClock, RecordingId, Rng, SeededRng, SessionId, SourceId, TzOffsetMin, UtcMillis,
};
use tinwave_db::{
    CheckpointPolicy, SignalStore, SignalWriter, StoreConfig, WriterConfig, WriterStats,
};
use tinwave_signals::{
    ActivityGuess, BusConfig, CompletedPayload, ContextKey, ContextScope, ContextSnapshotId,
    CounterEntity, CrossfadePayload, Daypart, ElapsedDs, EntityKind, EqPayload, EventContext,
    ExclusiveModePayload, ExplanationOpenedPayload, Fraction, GestureCoalescer, GestureTick,
    GestureTime, Hash32, InputSource, MetadataEditedPayload, MetadataFields, MutePayload,
    OutputClass, OutputDevicePayload, OutputTransport, PausedPayload, PlaceSlot, PlayOrigin,
    PlayStartedPayload, PlaylistEditPayload, PlaylistKind, PositionDs, QueueOrigin,
    QueueReorderedPayload, QueuedPayload, RadioFeedbackKind, RadioFeedbackPayload, Rate,
    RatePayload, RatedPayload, RemovedFromQueuePayload, RepeatMode, RepeatModePayload,
    ReplayedPayload, ResumedPayload, SearchNoResultPayload, SearchPerformedPayload,
    SearchResultOpenedPayload, SearchScope, Section, SeekPayload, SessionEndedPayload,
    SessionStartedPayload, SignalBus, SignalEvent, SignalKind, SignalPayload, SignalSettings,
    SkipPayload, Stars, SystemVolumePayload, TemperatureBand, View, ViewOpenedPayload, Volume,
    VolumePayload, WeatherCondition, WeekdayType, WindowStateKind, WindowStatePayload,
};

use crate::common::{Ids, TempDb, T0};

/// What to simulate.
#[derive(Debug, Clone, Copy)]
pub struct GrowthConfig {
    /// Seed of every random choice (and of the ids).
    pub seed: u64,
    /// Simulated days.
    pub days: u32,
    /// Plays per day (the reference heavy user: 120, 8 h/day).
    pub plays_per_day: u32,
    /// Tracks in the library (the reference user: 50 000).
    pub library_tracks: usize,
    /// Test hook: stall the producer for 3 ms after every `Paused` event, as a preempted thread
    /// would be. It must not change the stored rows (the lock step in the module docs); it is how
    /// the determinism test proves that. Off by default and never read from the environment.
    pub preempt_after_pause: bool,
}

impl GrowthConfig {
    /// The reference heavy user of `storage-budget.md` for `days` days.
    pub const fn reference(days: u32) -> Self {
        Self {
            seed: 2026,
            days,
            plays_per_day: 120,
            library_tracks: 50_000,
            preempt_after_pause: false,
        }
    }
}

/// The file size after a simulated day.
#[derive(Debug, Clone, Copy)]
pub struct SizeSample {
    pub day: u32,
    pub file_bytes: u64,
}

/// What a harness hook sees at the end of a simulated day (writer idle, everything committed).
pub struct DayEnd<'a> {
    pub day: u32,
    pub db_path: &'a Path,
}

/// How many raw ticks went in, to show what coalescing did.
#[derive(Debug, Clone, Copy, Default)]
pub struct TicksFed {
    pub volume: u64,
    pub system_volume: u64,
    pub rate: u64,
    pub seek: u64,
    /// Counter interactions (impressions, hovers, dwells).
    pub counter_interactions: u64,
}

/// A finished simulation: the database and what the pipeline reported.
pub struct Run {
    pub config: GrowthConfig,
    pub db: TempDb,
    pub writer: WriterStats,
    pub dropped: u64,
    pub reserved_overflows: u64,
    pub keys_dropped: u64,
    pub suppressed_gestures: u64,
    pub plays: u64,
    pub ticks: TicksFed,
    pub samples: Vec<SizeSample>,
    pub context_snapshots: u64,
    pub elapsed: Duration,
    /// Time the harness spent waiting for the writer at barriers.
    pub barrier_wait: Duration,
}

/// Local-clock offset the bus and writer use (so `ui_counter.day` is the local day).
const TZ_MINUTES: i16 = 60;
const DAY_MS: i64 = 86_400_000;

/// The barrier polls the writer's counters, sleeping 50 us first and doubling up to 500 us: a
/// commit takes about half a millisecond, so a wait costs at most a few polls and ends about
/// `BARRIER_POLL_MAX` after the commit (the OS sleep floor, ~0.55 ms on Windows, and load can make
/// it longer); the 60 s hang assert in `barrier` still bounds the wait.
const BARRIER_POLL_FIRST: Duration = Duration::from_micros(50);
const BARRIER_POLL_MAX: Duration = Duration::from_micros(500);

/// Deterministic dice on a `SeededRng`.
struct Dice(std::cell::RefCell<SeededRng>);

impl Dice {
    fn next(&self) -> u64 {
        let mut bytes = [0u8; 8];
        self.0.borrow_mut().fill_bytes(&mut bytes).unwrap();
        u64::from_le_bytes(bytes)
    }

    /// Uniform in `0..n`.
    fn below(&self, n: u64) -> u64 {
        self.next() % n.max(1)
    }

    /// Uniform in `lo..=hi`.
    fn range(&self, lo: u64, hi: u64) -> u64 {
        lo + self.below(hi - lo + 1)
    }

    /// True with probability `permille` / 1000.
    fn chance(&self, permille: u64) -> bool {
        self.below(1000) < permille
    }

    /// An index in `0..n` skewed toward 0 (a few favourites, a long tail).
    fn skewed(&self, n: usize) -> usize {
        let u = self.below(1_000_000);
        usize::try_from(u * u * n as u64 / (1_000_000 * 1_000_000)).unwrap()
    }
}

/// Things a play can contain besides its start and end, with the expected number per day
/// (catalog section 9 rates for 120 plays). Each is rolled once per play at `n / 120`.
const PER_DAY: &[(Action, u64)] = &[
    (Action::Volume, 35),
    (Action::SystemVolume, 12),
    (Action::Mute, 5),
    (Action::Rate, 5),
    (Action::Seek, 35),
    (Action::Replay, 8),
    (Action::Previous, 8),
    (Action::Repeat, 2),
    (Action::PauseResume, 4),
    (Action::QueueNext, 8),
    (Action::QueueLater, 12),
    (Action::RemoveFromQueue, 3),
    (Action::Reorder, 3),
    (Action::Favorite, 5),
    (Action::Unfavorite, 1),
    (Action::Rate5, 2),
    (Action::PlaylistAdd, 2),
    (Action::PlaylistRemove, 1),
    (Action::EditTags, 2),
    (Action::Artwork, 1),
    (Action::Search, 10),
    (Action::Window, 20),
    (Action::OutputDevice, 2),
    (Action::Eq, 5),
    (Action::Exclusive, 1),
    (Action::Crossfade, 2),
    (Action::View, 15),
    (Action::RadioFeedback, 3),
    (Action::Explanation, 2),
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Action {
    Volume,
    SystemVolume,
    Mute,
    Rate,
    Seek,
    Replay,
    Previous,
    Repeat,
    PauseResume,
    QueueNext,
    QueueLater,
    RemoveFromQueue,
    Reorder,
    Favorite,
    Unfavorite,
    Rate5,
    PlaylistAdd,
    PlaylistRemove,
    EditTags,
    Artwork,
    Search,
    Window,
    OutputDevice,
    Eq,
    Exclusive,
    Crossfade,
    View,
    RadioFeedback,
    Explanation,
}

type Track = (RecordingId, SourceId);

struct Sim {
    cfg: GrowthConfig,
    /// The harness's own connection, for the context snapshot rows (see `insert_pending_snapshots`).
    snapshot_conn: Connection,
    dice: Dice,
    clock: ManualClock,
    ids: Ids,
    bus: SignalBus,
    writer: Option<SignalWriter>,
    ui: GestureCoalescer,
    sys: GestureCoalescer,
    library: Vec<Track>,
    albums: Vec<[u8; 16]>,
    artists: Vec<[u8; 16]>,
    playlists: Vec<[u8; 16]>,
    impression_set: Vec<CounterEntity>,
    hover_set: Vec<CounterEntity>,
    dwell_set: Vec<CounterEntity>,
    // State of the listener.
    session: SessionId,
    session_plays: u32,
    current: Track,
    ctx: u64,
    pending_snapshots: Vec<u64>,
    volume: u8,
    system_volume: u8,
    rate: u8,
    // Accounting.
    barrier_wait: Duration,
    ticks: TicksFed,
    plays: u64,
    snapshots: u64,
}

impl Sim {
    fn new(cfg: GrowthConfig, db: &TempDb) -> Self {
        let mut ids = Ids::new(cfg.seed);
        let device: DeviceId = ids.device();
        let library: Vec<Track> = (0..cfg.library_tracks)
            .map(|_| (ids.recording(), ids.source()))
            .collect();
        let uuids = |ids: &mut Ids, n: usize| -> Vec<[u8; 16]> {
            (0..n).map(|_| *ids.recording().as_bytes()).collect()
        };
        let albums = uuids(&mut ids, cfg.library_tracks / 10);
        let artists = uuids(&mut ids, cfg.library_tracks / 25);
        let playlists = uuids(&mut ids, 40);

        // Production folds the WAL into the file every 8 MiB when idle; every checkpoint fsyncs
        // (100+ ms on Windows) and the harness is idle at every barrier, so it would spend most of
        // its time checkpointing. A larger WAL changes when pages move, not what the file holds.
        let store_config = StoreConfig {
            checkpoint: CheckpointPolicy {
                soft_limit_bytes: 96 * 1024 * 1024,
                hard_limit_bytes: 256 * 1024 * 1024,
                ..CheckpointPolicy::default()
            },
            ..StoreConfig::default()
        };
        let store = SignalStore::open_with(&db.path, store_config).unwrap();
        let mut bus_config = BusConfig::new(device);
        bus_config.first_seq = store.first_seq(device).unwrap();
        bus_config.tz_offset = TzOffsetMin::new(TZ_MINUTES).unwrap();
        let (bus, drain) = SignalBus::channel(
            bus_config,
            ids.clock.clone(),
            Arc::new(SignalSettings::new()),
        )
        .unwrap();
        // Only play ends (and 256 events) flush: the harness's barrier is the play end.
        let writer_config = WriterConfig {
            flush_interval: Duration::from_secs(600),
            ..WriterConfig::default()
        };
        let writer = SignalWriter::spawn(store, drain, writer_config).unwrap();
        writer.set_tz_offset(TzOffsetMin::new(TZ_MINUTES).unwrap());
        let session = ids.session();
        let current = library[0];
        Self {
            snapshot_conn: {
                // The writer has migrated the file by now. Not fsynced: only the final size matters.
                let conn = Connection::open(&db.path).unwrap();
                conn.busy_timeout(Duration::from_secs(10)).unwrap();
                conn.pragma_update(None, "synchronous", "OFF").unwrap();
                conn
            },
            dice: Dice(std::cell::RefCell::new(SeededRng::new(cfg.seed ^ 0x5EED))),
            clock: ids.clock.clone(),
            ui: GestureCoalescer::new(bus.clone()),
            sys: GestureCoalescer::new(bus.clone()),
            cfg,
            ids,
            bus,
            writer: Some(writer),
            library,
            albums,
            artists,
            playlists,
            impression_set: Vec::new(),
            hover_set: Vec::new(),
            dwell_set: Vec::new(),
            session,
            session_plays: 0,
            current,
            ctx: 1,
            pending_snapshots: vec![1],
            volume: 128,
            system_volume: 140,
            rate: 50,
            barrier_wait: Duration::ZERO,
            ticks: TicksFed::default(),
            plays: 0,
            snapshots: 1,
        }
    }

    fn now(&self) -> i64 {
        use tinwave_core::Clock;
        self.clock.now().as_i64()
    }

    /// Moves the clock forward (never back) and lets finished gestures end.
    fn at(&mut self, t: i64) {
        if t > self.now() {
            self.clock.set(UtcMillis::new(t));
        }
        self.ui.flush_due();
        self.sys.flush_due();
    }

    fn advance(&mut self, ms: i64) {
        let t = self.now() + ms;
        self.at(t);
    }

    fn pick_track(&mut self) -> Track {
        let i = self.dice.skewed(self.library.len());
        self.library[i]
    }

    /// A context snapshot changes about every 8 events (~60 a day): a new id, whose row is
    /// inserted at the next barrier.
    fn maybe_new_context(&mut self) {
        if self.dice.chance(150) {
            self.ctx += 1;
            self.snapshots += 1;
            self.pending_snapshots.push(self.ctx);
        }
    }

    fn event_context(&mut self, pos_ds: Option<u32>) -> EventContext {
        EventContext {
            session: self.session,
            recording: Some(self.current.0),
            source: Some(self.current.1),
            pos_ds,
            ctx: Some(ContextSnapshotId(self.ctx)),
        }
    }

    /// Emits one event through the bus. `track` is the recording it is about (`None` for
    /// events about the app).
    fn emit(&mut self, track: Option<Track>, pos_ds: Option<u32>, payload: SignalPayload) {
        self.maybe_new_context();
        // The row exists before an event names it, as a context hub that attaches an id would have
        // stored it: the play record reads its key from this row.
        self.insert_pending_snapshots();
        let session = self.session;
        let ctx = ContextSnapshotId(self.ctx);
        let outcome = self.bus.emit_with(payload.kind(), |stamp| SignalEvent {
            id: stamp.id,
            ts: stamp.ts,
            session,
            recording: track.map(|t| t.0),
            source: track.map(|t| t.1),
            pos_ds,
            payload,
            ctx: Some(ctx),
        });
        let outcome = outcome.unwrap();
        assert_eq!(
            outcome,
            tinwave_signals::EmitOutcome::Queued,
            "{:?} was not queued",
            payload.kind()
        );
        // Events that end a play make the writer commit at once: wait for it (module docs).
        if tinwave_db::ends_a_play(payload.kind()) {
            if self.cfg.preempt_after_pause && payload.kind() == SignalKind::Paused {
                std::thread::sleep(Duration::from_millis(3));
            }
            self.barrier();
        }
    }

    /// Emits an event about the current track.
    fn emit_here(&mut self, pos_ds: u32, payload: SignalPayload) {
        let track = self.current;
        self.emit(Some(track), Some(pos_ds), payload);
    }

    // ---- the day -------------------------------------------------------------------------

    fn run_day(&mut self, day: u32) {
        let day_start = T0 + i64::from(day) * DAY_MS;
        self.new_day_working_set();
        // 120 plays +- 10 %, in three sessions (morning, afternoon, evening).
        let total = u64::from(self.cfg.plays_per_day);
        let plays_today = self.dice.range(total * 9 / 10, total * 11 / 10);
        let split = [plays_today / 4, plays_today * 3 / 8];
        let counts = [split[0], split[1], plays_today - split[0] - split[1]];
        let starts_h = [7, 12, 18];
        for (session_index, plays) in counts.into_iter().enumerate() {
            let start = day_start
                + (starts_h[session_index] * 3_600_000)
                + self.dice.below(45 * 60_000) as i64;
            self.at(start);
            self.begin_session();
            for n in 0..plays {
                let last = n + 1 == plays;
                self.play(last, plays);
                self.barrier();
            }
        }
    }

    fn begin_session(&mut self) {
        self.ui.flush_all();
        self.sys.flush_all();
        self.session = self.ids.session();
        self.session_plays = 0;
    }

    /// One play: counters first, then the events, ending with a play-end event.
    fn play(&mut self, ends_session: bool, session_plays: u64) {
        self.counters_for_a_play();
        if self.session_plays == 0 {
            self.emit(
                None,
                None,
                SignalPayload::SessionStarted(SessionStartedPayload {
                    start_origin: PlayOrigin::Resume,
                }),
            );
        }
        self.session_plays += 1;
        self.plays += 1;
        self.current = self.pick_track();

        let start = self.now();
        let length_ms = i64::try_from(self.dice.range(180_000, 360_000)).unwrap();
        let skipped = self.dice.chance(300);
        let ends_after = if skipped {
            i64::try_from(self.dice.range(4_000, length_ms as u64 - 10_000)).unwrap()
        } else {
            length_ms
        };
        let origin = match self.dice.below(10) {
            0 => PlayOrigin::Search,
            1 => PlayOrigin::Album,
            2 => PlayOrigin::Playlist,
            3..=5 => PlayOrigin::Radio,
            _ => PlayOrigin::Queue,
        };
        let input = if matches!(origin, PlayOrigin::Radio | PlayOrigin::Queue) {
            InputSource::Auto
        } else {
            InputSource::Mouse
        };
        self.emit_here(
            0,
            SignalPayload::PlayStarted(PlayStartedPayload { origin, input }),
        );

        // The other things that happen during the play, each with its daily rate / 120.
        let mut actions: Vec<(i64, Action)> = Vec::new();
        for &(action, per_day) in PER_DAY {
            // `per_day` may exceed one per play for some actions; roll the whole part and the rest.
            let permille = per_day * 1000 / 120;
            let mut roll = permille;
            while roll > 0 {
                if self.dice.chance(roll.min(1000)) {
                    let offset =
                        i64::try_from(self.dice.range(1_500, ends_after as u64 - 1_000)).unwrap();
                    actions.push((start + offset, action));
                }
                roll = roll.saturating_sub(1000);
            }
        }
        actions.sort_by_key(|a| a.0);
        for (t, action) in actions {
            self.at(t);
            let elapsed_ds = u32::try_from(((self.now() - start) / 100).max(0)).unwrap();
            self.do_action(action, elapsed_ds);
        }

        // The track changes: every gesture in progress ends before the end event.
        self.at(start + ends_after);
        self.ui.flush_all();
        self.sys.flush_all();
        let pos_ds = u32::try_from(ends_after / 100).unwrap();
        if skipped {
            self.emit_here(
                pos_ds,
                SignalPayload::Skipped(SkipPayload {
                    fraction: Fraction::from_raw(
                        u8::try_from(ends_after * 255 / length_ms).unwrap(),
                    ),
                    input: InputSource::Mouse,
                    section: Section::UNKNOWN,
                    queue_origin: if matches!(origin, PlayOrigin::Radio) {
                        QueueOrigin::Radio
                    } else {
                        QueueOrigin::Manual
                    },
                }),
            );
        } else {
            let paused_ds = PositionDs::new(u32::try_from(self.dice.below(200)).unwrap()).unwrap();
            self.emit_here(
                pos_ds,
                SignalPayload::Completed(CompletedPayload {
                    listened_ds: PositionDs::new(pos_ds).unwrap(),
                    paused_ds,
                }),
            );
        }
        if ends_session {
            self.emit(
                None,
                None,
                SignalPayload::SessionEnded(SessionEndedPayload {
                    tracks: u16::try_from(session_plays).unwrap(),
                    minutes: u16::try_from(session_plays * 4).unwrap(),
                }),
            );
        }
    }

    fn do_action(&mut self, action: Action, elapsed_ds: u32) {
        match action {
            Action::Volume => self.volume_gesture(elapsed_ds),
            Action::SystemVolume => self.system_volume_gesture(elapsed_ds, false),
            Action::Mute => {
                let muted = self.dice.chance(500);
                self.emit_here(
                    elapsed_ds,
                    SignalPayload::MuteToggled(MutePayload {
                        muted,
                        input: InputSource::Keyboard,
                    }),
                );
            }
            Action::Rate => self.rate_gesture(),
            Action::Seek => self.seek_gesture(elapsed_ds),
            Action::Replay => {
                let loop_count = u8::try_from(self.dice.range(1, 3)).unwrap();
                self.emit_here(
                    elapsed_ds,
                    SignalPayload::Replayed(ReplayedPayload { loop_count }),
                );
            }
            Action::Previous => self.emit_here(elapsed_ds, SignalPayload::PreviousPressed),
            Action::Repeat => {
                let mode = [RepeatMode::Off, RepeatMode::All, RepeatMode::One]
                    [self.dice.below(3) as usize];
                self.emit_here(
                    elapsed_ds,
                    SignalPayload::RepeatModeChanged(RepeatModePayload { mode }),
                );
            }
            Action::PauseResume => {
                self.emit_here(
                    elapsed_ds,
                    SignalPayload::Paused(PausedPayload {
                        input: InputSource::Keyboard,
                    }),
                );
                let paused_ms = i64::try_from(self.dice.range(3_000, 90_000)).unwrap();
                self.advance(paused_ms);
                self.emit_here(
                    elapsed_ds,
                    SignalPayload::Resumed(ResumedPayload {
                        pause_ds: PositionDs::new(u32::try_from(paused_ms / 100).unwrap()).unwrap(),
                        input: InputSource::Keyboard,
                    }),
                );
            }
            Action::QueueNext | Action::QueueLater => {
                let queued = self.pick_track();
                let payload = QueuedPayload {
                    from_view: [View::Library, View::Album, View::Search, View::Radio]
                        [self.dice.below(4) as usize],
                };
                let payload = if action == Action::QueueNext {
                    SignalPayload::QueuedNext(payload)
                } else {
                    SignalPayload::QueuedLater(payload)
                };
                self.emit(Some(queued), None, payload);
            }
            Action::RemoveFromQueue => {
                let removed = self.pick_track();
                let was_radio_pick = self.dice.chance(500);
                self.emit(
                    Some(removed),
                    None,
                    SignalPayload::RemovedFromQueue(RemovedFromQueuePayload { was_radio_pick }),
                );
            }
            Action::Reorder => {
                let moved = self.pick_track();
                let distance = i16::try_from(self.dice.range(1, 12)).unwrap()
                    * if self.dice.chance(500) { 1 } else { -1 };
                self.emit(
                    Some(moved),
                    None,
                    SignalPayload::QueueReordered(QueueReorderedPayload { distance }),
                );
            }
            Action::Favorite => self.emit_here(elapsed_ds, SignalPayload::Favorited),
            Action::Unfavorite => self.emit_here(elapsed_ds, SignalPayload::Unfavorited),
            Action::Rate5 => {
                let stars = Stars::new(u8::try_from(self.dice.range(0, 5)).unwrap()).unwrap();
                self.emit_here(elapsed_ds, SignalPayload::Rated(RatedPayload { stars }));
            }
            Action::PlaylistAdd | Action::PlaylistRemove => {
                // `playlist_id` is a `dict_entity` id in production; any small id has the same size.
                let payload = PlaylistEditPayload {
                    playlist_id: u32::try_from(self.dice.range(1, 40)).unwrap(),
                    playlist_kind: PlaylistKind::Manual,
                };
                let payload = if action == Action::PlaylistAdd {
                    SignalPayload::PlaylistAdded(payload)
                } else {
                    SignalPayload::PlaylistRemoved(payload)
                };
                self.emit_here(elapsed_ds, payload);
            }
            Action::EditTags => {
                let bits = 1u16 << self.dice.below(8);
                self.emit_here(
                    elapsed_ds,
                    SignalPayload::MetadataEdited(MetadataEditedPayload {
                        fields: MetadataFields::from_bits(bits).unwrap(),
                    }),
                );
            }
            Action::Artwork => self.emit_here(elapsed_ds, SignalPayload::ArtworkChanged),
            Action::Search => self.search(),
            Action::Window => {
                let state = [
                    WindowStateKind::Focused,
                    WindowStateKind::Unfocused,
                    WindowStateKind::Minimized,
                    WindowStateKind::Tray,
                ][self.dice.below(4) as usize];
                self.emit(
                    None,
                    None,
                    SignalPayload::WindowState(WindowStatePayload { state }),
                );
            }
            Action::OutputDevice => {
                let class = [
                    OutputClass::Headphones,
                    OutputClass::Speakers,
                    OutputClass::Bluetooth,
                    OutputClass::UsbDac,
                ][self.dice.below(4) as usize];
                self.emit(
                    None,
                    None,
                    SignalPayload::OutputDeviceChanged(OutputDevicePayload {
                        class,
                        transport: OutputTransport::Usb,
                        form_factor: u8::try_from(self.dice.below(11)).unwrap(),
                        device_hash: Hash32(self.dice.next() as u32),
                    }),
                );
            }
            Action::Eq => self.emit(
                None,
                None,
                SignalPayload::EqChanged(EqPayload {
                    preset: u8::try_from(self.dice.below(8)).unwrap(),
                    bass_db: i8::try_from(self.dice.range(0, 12)).unwrap() - 6,
                    treble_db: i8::try_from(self.dice.range(0, 12)).unwrap() - 6,
                }),
            ),
            Action::Exclusive => {
                let on = self.dice.chance(500);
                self.emit(
                    None,
                    None,
                    SignalPayload::ExclusiveModeToggled(ExclusiveModePayload { on }),
                );
            }
            Action::Crossfade => self.emit(
                None,
                None,
                SignalPayload::CrossfadeChanged(CrossfadePayload {
                    crossfade_ds: u8::try_from(self.dice.range(0, 120)).unwrap(),
                }),
            ),
            Action::View => {
                let view = [
                    View::Lyrics,
                    View::NowPlaying,
                    View::MiniPlayer,
                    View::Insights,
                ][self.dice.below(4) as usize];
                self.emit(
                    None,
                    None,
                    SignalPayload::ViewOpened(ViewOpenedPayload { view }),
                );
            }
            Action::RadioFeedback => {
                let kind = [
                    RadioFeedbackKind::NotNow,
                    RadioFeedbackKind::MoreLikeThis,
                    RadioFeedbackKind::NeverInContext,
                    RadioFeedbackKind::ForgetThisPlay,
                ][self.dice.below(4) as usize];
                self.emit_here(
                    elapsed_ds,
                    SignalPayload::RadioFeedback(RadioFeedbackPayload {
                        kind,
                        context_scope: ContextScope::from_bits(
                            u16::try_from(self.dice.below(8)).unwrap(),
                        )
                        .unwrap(),
                    }),
                );
            }
            Action::Explanation => self.emit_here(
                elapsed_ds,
                SignalPayload::ExplanationOpened(ExplanationOpenedPayload {
                    reason_kind: u8::try_from(self.dice.below(12)).unwrap(),
                }),
            ),
        }
    }

    fn search(&mut self) {
        let query_len = u8::try_from(self.dice.range(2, 24)).unwrap();
        let query_hash = Hash32(self.dice.next() as u32);
        let no_result = self.dice.chance(300);
        self.emit(
            None,
            None,
            SignalPayload::SearchPerformed(SearchPerformedPayload {
                query_len,
                query_hash,
                result_count: if no_result {
                    0
                } else {
                    u16::try_from(self.dice.range(1, 400)).unwrap()
                },
                scope: SearchScope::All,
            }),
        );
        self.advance(2_000);
        if no_result {
            self.emit(
                None,
                None,
                SignalPayload::SearchNoResult(SearchNoResultPayload { query_hash }),
            );
        } else if self.dice.chance(800) {
            let opened = self.pick_track();
            self.emit(
                Some(opened),
                None,
                SignalPayload::SearchResultOpened(SearchResultOpenedPayload {
                    rank: u8::try_from(self.dice.range(0, 9)).unwrap(),
                    entity_kind: EntityKind::Track,
                }),
            );
        }
    }

    // ---- gestures, fed tick by tick to the real coalescers --------------------------------

    fn volume_gesture(&mut self, elapsed_ds: u32) {
        let steps = self.dice.range(4, 24);
        let target = u8::try_from(self.dice.range(40, 230)).unwrap();
        let spacing = i64::try_from(self.dice.range(20, 60)).unwrap();
        let context = self.event_context(Some(elapsed_ds));
        let start_volume = self.volume;
        for i in 1..=steps {
            let from = self.volume;
            let to = u8::try_from(
                (i64::from(start_volume)
                    + (i64::from(target) - i64::from(start_volume)) * i64::try_from(i).unwrap()
                        / i64::try_from(steps).unwrap())
                .clamp(0, 255),
            )
            .unwrap();
            self.advance(spacing);
            let context = EventContext {
                ctx: context.ctx,
                ..context
            };
            let _ = self.ui.volume(|| GestureTick {
                context,
                payload: VolumePayload {
                    from: Volume::from_raw(from),
                    to: Volume::from_raw(to),
                    gesture: GestureTime::from_ms(u32::try_from(spacing).unwrap()),
                    input: InputSource::Mouse,
                    section: Section::UNKNOWN,
                    since_track_start: ElapsedDs::from_ds(
                        u16::try_from(elapsed_ds.min(65_535)).unwrap(),
                    ),
                    since_section_start: ElapsedDs::from_ds(
                        u16::try_from(elapsed_ds.min(600)).unwrap(),
                    ),
                },
            });
            self.ticks.volume += 1;
            self.volume = to;
        }
        // Silence long enough for the gesture to end.
        self.advance(700);
    }

    /// An OS volume gesture: the keyboard volume keys or the mixer. A `burst` is the P-0010
    /// case: 60 steps within 200 ms.
    fn system_volume_gesture(&mut self, elapsed_ds: u32, burst: bool) {
        let (steps, spacing) = if burst || self.dice.chance(150) {
            (60u64, 3i64)
        } else {
            (
                self.dice.range(2, 8),
                i64::try_from(self.dice.range(40, 120)).unwrap(),
            )
        };
        let context = self.event_context(Some(elapsed_ds));
        let up = self.system_volume < 200 && self.dice.chance(500);
        for _ in 0..steps {
            let from = self.system_volume;
            let to = if up {
                from.saturating_add(1)
            } else {
                from.saturating_sub(1)
            };
            self.advance(spacing);
            let _ = self.sys.system_volume(|| GestureTick {
                context,
                payload: SystemVolumePayload {
                    from: Volume::from_raw(from),
                    to: Volume::from_raw(to),
                    gesture: GestureTime::from_ms(0),
                    input: InputSource::System,
                    section: Section::UNKNOWN,
                    since_track_start: ElapsedDs::from_ds(
                        u16::try_from(elapsed_ds.min(65_535)).unwrap(),
                    ),
                    since_section_start: ElapsedDs::from_ds(
                        u16::try_from(elapsed_ds.min(600)).unwrap(),
                    ),
                    endpoint_class: OutputClass::Headphones,
                },
            });
            self.ticks.system_volume += 1;
            self.system_volume = to;
        }
        self.advance(500);
    }

    fn rate_gesture(&mut self) {
        let context = self.event_context(None);
        let target = u8::try_from(self.dice.range(25, 100)).unwrap();
        for _ in 0..self.dice.range(2, 4) {
            let from = self.rate;
            let to = if from < target {
                from + 5
            } else {
                from.saturating_sub(5)
            };
            self.advance(120);
            let _ = self.ui.rate(|| GestureTick {
                context,
                payload: RatePayload {
                    from: Rate::from_raw(from),
                    to: Rate::from_raw(to),
                    input: InputSource::Mouse,
                    section: Section::UNKNOWN,
                },
            });
            self.ticks.rate += 1;
            self.rate = to;
        }
        self.advance(700);
    }

    /// A seek: one to four drags of the seek bar, 0.2-0.9 s apart. About one gesture in eight is
    /// a nudge of 2 s or less, which the coalescer drops.
    fn seek_gesture(&mut self, elapsed_ds: u32) {
        let nudge = self.dice.chance(125);
        let drags = if nudge { 1 } else { self.dice.range(1, 4) };
        let context = self.event_context(Some(elapsed_ds));
        let mut position = elapsed_ds;
        for _ in 0..drags {
            let jump = if nudge {
                i64::try_from(self.dice.range(3, 15)).unwrap()
            } else {
                i64::try_from(self.dice.range(80, 900)).unwrap()
            };
            let back = !nudge && self.dice.chance(300);
            let to = u32::try_from(
                (i64::from(position) + if back { -jump } else { jump }).clamp(0, 2_400),
            )
            .unwrap();
            self.advance(i64::try_from(self.dice.range(200, 900)).unwrap());
            let from = position;
            let _ = self.ui.seek(|| GestureTick {
                context,
                payload: SeekPayload {
                    from_ds: PositionDs::new(from).unwrap(),
                    to_ds: PositionDs::new(to).unwrap(),
                    section_from: Section::UNKNOWN,
                    section_to: Section::UNKNOWN,
                    input: InputSource::Mouse,
                },
            });
            self.ticks.seek += 1;
            position = to;
        }
        self.advance(2_100);
    }

    // ---- UI counters ---------------------------------------------------------------------

    /// The things the listener looks at today: a working set per counter kind, drawn (skewed) from
    /// the library, so popular entities come back on other days. The sizes make about 300
    /// distinct `ui_counter` rows a day, the catalog's rate (section 9: "~300 rows").
    fn new_day_working_set(&mut self) {
        self.impression_set = (0..185).map(|_| self.counter_entity()).collect();
        self.hover_set = (0..75).map(|_| self.counter_entity()).collect();
        self.dwell_set = (0..40).map(|_| self.page_entity()).collect();
    }

    /// What the listener looked at while this play went on, for the counters (about 38
    /// interactions a play, 4 600 a day, spread over today's working set).
    fn counters_for_a_play(&mut self) {
        let bus = self.bus.clone();
        let counters = bus.counters();
        for _ in 0..28 {
            let i = self.dice.below(self.impression_set.len() as u64) as usize;
            let _ = counters.impression(self.impression_set[i]);
            self.ticks.counter_interactions += 1;
        }
        for _ in 0..8 {
            let i = self.dice.below(self.hover_set.len() as u64) as usize;
            let ms = u32::try_from(self.dice.range(800, 6_000)).unwrap();
            let _ = counters.artwork_hover(self.hover_set[i], ms);
            self.ticks.counter_interactions += 1;
        }
        for _ in 0..self.dice.range(1, 3) {
            let i = self.dice.below(self.dwell_set.len() as u64) as usize;
            let ms = u32::try_from(self.dice.range(3_000, 180_000)).unwrap();
            let scrolled = Fraction::from_raw(u8::try_from(self.dice.below(256)).unwrap());
            let _ = counters.page_dwell(self.dwell_set[i], ms, scrolled);
            self.ticks.counter_interactions += 1;
        }
    }

    fn counter_entity(&mut self) -> CounterEntity {
        match self.dice.below(100) {
            0..=44 => {
                let i = self.dice.skewed(self.library.len());
                CounterEntity::track(self.library[i].0)
            }
            45..=79 => {
                let i = self.dice.skewed(self.albums.len());
                CounterEntity::new(EntityKind::Album, self.albums[i])
            }
            80..=94 => {
                let i = self.dice.skewed(self.artists.len());
                CounterEntity::new(EntityKind::Artist, self.artists[i])
            }
            _ => {
                let i = self.dice.skewed(self.playlists.len());
                CounterEntity::new(EntityKind::Playlist, self.playlists[i])
            }
        }
    }

    /// Pages with a dwell: albums, artists, playlists.
    fn page_entity(&mut self) -> CounterEntity {
        match self.dice.below(100) {
            0..=59 => {
                let i = self.dice.skewed(self.albums.len());
                CounterEntity::new(EntityKind::Album, self.albums[i])
            }
            60..=89 => {
                let i = self.dice.skewed(self.artists.len());
                CounterEntity::new(EntityKind::Artist, self.artists[i])
            }
            _ => {
                let i = self.dice.skewed(self.playlists.len());
                CounterEntity::new(EntityKind::Playlist, self.playlists[i])
            }
        }
    }

    // ---- lock step with the writer --------------------------------------------------------

    /// Waits until the writer has committed every event the bus accepted. The last event of a
    /// tick ends a play, which the writer flushes at once, so this returns within a commit.
    ///
    /// The wait sleeps (see [`BARRIER_POLL_FIRST`]) instead of spinning: the writer publishes its
    /// progress in plain atomics and has nothing to signal, and a spinning producer would take a
    /// whole core, and more wall clock under load, from the writer it is waiting for.
    fn barrier(&mut self) {
        let waiting = Instant::now();
        let target = self.bus.queued_total();
        let writer = self.writer.as_ref().unwrap();
        let start = Instant::now();
        let mut pause = BARRIER_POLL_FIRST;
        loop {
            let s = writer.stats();
            if s.events_inserted + s.events_duplicate + s.events_rejected >= target {
                self.barrier_wait += waiting.elapsed();
                return;
            }
            assert!(
                start.elapsed() < Duration::from_secs(60),
                "writer did not commit {target} events: {s:?}, last error {:?}",
                writer.take_last_error().map(|e| e.chain())
            );
            std::thread::sleep(pause);
            pause = (pause * 2).min(BARRIER_POLL_MAX);
        }
    }

    /// A context as the providers would report it: the clock fields always, the others mostly.
    fn random_context_key(&self) -> ContextKey {
        let pick = |n: usize| usize::try_from(self.dice.below(n as u64)).unwrap();
        let mut key = ContextKey::UNKNOWN
            .with_daypart(Daypart::ALL[pick(Daypart::ALL.len())])
            .with_weekday_type(WeekdayType::ALL[pick(WeekdayType::ALL.len())]);
        if self.dice.chance(700) {
            key = key
                .with_condition(WeatherCondition::ALL[pick(WeatherCondition::ALL.len())])
                .with_temperature(TemperatureBand::ALL[pick(TemperatureBand::ALL.len())]);
        }
        if self.dice.chance(900) {
            key = key.with_device_class(OutputClass::ALL[pick(OutputClass::ALL.len())]);
        }
        if self.dice.chance(800) {
            key = key.with_activity(ActivityGuess::ALL[pick(ActivityGuess::ALL.len())]);
        }
        if self.dice.chance(500) {
            key = key.with_place(PlaceSlot::new(1 + u8::try_from(pick(8)).unwrap()).unwrap());
        }
        key
    }

    /// Context snapshot rows for the ids created since the last call. The context hub does not
    /// exist yet, so the harness writes the rows the schema defines (`context_snapshot`): a 64-bit
    /// hash, a valid packed [`ContextKey`] (the writer reads it into the play record) and ~30
    /// bytes of quantized fields (`context-engine.md`). The writer may be committing a batch of
    /// 256 events meanwhile; this connection waits for it (busy timeout).
    fn insert_pending_snapshots(&mut self) {
        if self.pending_snapshots.is_empty() {
            return;
        }
        let ids = std::mem::take(&mut self.pending_snapshots);
        let rows: Vec<(u64, i64, u32, Vec<u8>)> = ids
            .into_iter()
            .map(|id| {
                let hash = i64::from_ne_bytes(self.dice.next().to_ne_bytes());
                let key = self.random_context_key().raw();
                let mut fields = vec![0u8; 30];
                for chunk in fields.chunks_mut(8) {
                    let bytes = self.dice.next().to_le_bytes();
                    chunk.copy_from_slice(&bytes[..chunk.len()]);
                }
                (id, hash, key, fields)
            })
            .collect();
        let tx = self.snapshot_conn.unchecked_transaction().unwrap();
        for (id, hash, key, fields) in rows {
            tx.execute(
                "INSERT INTO context_snapshot (id, hash, key, fields) VALUES (?1, ?2, ?3, ?4)",
                params![i64::try_from(id).unwrap(), hash, key, fields],
            )
            .unwrap();
        }
        tx.commit().unwrap();
    }
}

/// Runs the simulation and returns the finished database.
pub fn run(cfg: GrowthConfig, on_day_end: &mut dyn FnMut(&DayEnd<'_>)) -> Run {
    let started = Instant::now();
    let db = TempDb::new();
    let mut sim = Sim::new(cfg, &db);
    let mut samples = Vec::new();
    let reader = Connection::open(&db.path).unwrap();
    for day in 0..cfg.days {
        let db_path = db.path.clone();
        sim.run_day(day);
        // The tail of the day: every gesture ended, everything committed.
        sim.barrier();
        on_day_end(&DayEnd {
            day,
            db_path: &db_path,
        });
        if day % 10 == 9 || day + 1 == cfg.days {
            let pages: i64 = reader
                .query_row("PRAGMA page_count", [], |r| r.get(0))
                .unwrap();
            let free: i64 = reader
                .query_row("PRAGMA freelist_count", [], |r| r.get(0))
                .unwrap();
            let page_size: i64 = reader
                .query_row("PRAGMA page_size", [], |r| r.get(0))
                .unwrap();
            samples.push(SizeSample {
                day: day + 1,
                file_bytes: u64::try_from((pages - free) * page_size).unwrap(),
            });
        }
    }
    drop(reader);
    let suppressed = sim.ui.suppressed() + sim.sys.suppressed();
    let (plays, ticks, snapshots) = (sim.plays, sim.ticks, sim.snapshots);
    let barrier_wait = sim.barrier_wait;
    let (dropped, overflows, keys_dropped) = (
        sim.bus.dropped_total(),
        sim.bus.reserved_overflows(),
        sim.bus.counters().keys_dropped(),
    );
    // Gestures and counters still pending belong to the last tick; end them and flush.
    sim.ui.flush_all();
    sim.sys.flush_all();
    let writer = sim.writer.take().unwrap().shutdown().unwrap();
    // Fold the WAL into the file so page counts are the file's.
    let conn = Connection::open(&db.path).unwrap();
    conn.query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |_| Ok(()))
        .unwrap();
    drop(conn);
    Run {
        config: cfg,
        db,
        writer,
        dropped,
        reserved_overflows: overflows,
        keys_dropped,
        suppressed_gestures: suppressed,
        plays,
        ticks,
        samples,
        context_snapshots: snapshots,
        elapsed: started.elapsed(),
        barrier_wait,
    }
}

/// Counts of stored events per kind (for the report).
pub fn counts_by_kind(conn: &Connection) -> BTreeMap<u8, u64> {
    let mut stmt = conn
        .prepare("SELECT kind, count(*) FROM signal_event GROUP BY kind")
        .unwrap();
    stmt.query_map([], |r| Ok((r.get::<_, u8>(0)?, r.get::<_, i64>(1)? as u64)))
        .unwrap()
        .map(Result::unwrap)
        .collect()
}
