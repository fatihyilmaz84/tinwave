//! Storage growth test harness (ADR 0002, `docs/rules/testing.md` "Storage growth test").
//!
//! [`sim`] generates the reference heavy listener and drives it through the real pipeline;
//! this module measures the resulting `signals.db` and compares it with the volume table of
//! `docs/guides/signal-catalog.md` section 9.
//!
//! # How bytes are attributed to signal groups
//!
//! SQLite stores rows in b-tree pages, so "the bytes of one kind" is not a number the file has.
//! The harness computes each `signal_event` row's record size from the SQLite file format
//! (header, one serial type per column, integers in 0/1/2/3/4/6/8 bytes by magnitude, blob length,
//! cell length and cell pointer) and scales the sum of a group's rows by
//! `measured table bytes / sum of all rows`, where the measured table bytes are the table's
//! pages from `dbstat`. The groups therefore add up to the real table size, page fill included.
//! `ui_counter` and `context_snapshot` are whole tables and need no attribution.

#![allow(dead_code)]

pub mod sim;

use std::collections::BTreeMap;
use std::fmt::Write as _;

use rusqlite::Connection;
use tinwave_signals::SignalKind;

pub use sim::{run, GrowthConfig, Run};

/// The catalog counts a kilobyte as 1 000 bytes here (the smaller reading, so the test is the
/// stricter one).
const KB: f64 = 1000.0;

/// How far above the catalog a group may be: +50 %.
pub const TOLERANCE: f64 = 1.5;

/// The warm tier and cold tier the catalog expects, per year, in MB (play records ~2, monthly
/// rollups ~0.5). They are not built yet (T-0029, compaction), so the extrapolation takes the
/// catalog's own numbers for them. It also keeps the dictionary tables at their 90-day size,
/// although compaction does not remove them and they grow until the library is covered (a few
/// MB more, still far under the 60 MB budget).
pub const CATALOG_WARM_MB_PER_YEAR: f64 = 2.0;
pub const CATALOG_COLD_MB_PER_YEAR: f64 = 0.5;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Group {
    PlayStartEnd,
    VolumeMuteSpeed,
    SeekReplay,
    QueueLibrary,
    Search,
    OutputEqWindow,
    /// Event kinds that section 9 does not list (views, session markers, radio feedback).
    NotInCatalog,
}

impl Group {
    pub const ALL: [Self; 7] = [
        Self::PlayStartEnd,
        Self::VolumeMuteSpeed,
        Self::SeekReplay,
        Self::QueueLibrary,
        Self::Search,
        Self::OutputEqWindow,
        Self::NotInCatalog,
    ];

    pub const fn name(self) -> &'static str {
        match self {
            Self::PlayStartEnd => "Play start/end, skips, completes",
            Self::VolumeMuteSpeed => "Volume, mute, speed (coalesced)",
            Self::SeekReplay => "Seeks, replays, previous, repeat, pause",
            Self::QueueLibrary => "Queue, favorites, ratings, playlists, edits",
            Self::Search => "Search",
            Self::OutputEqWindow => "Output/EQ/window state changes",
            Self::NotInCatalog => "Not in section 9 (views, sessions, radio)",
        }
    }

    /// Section 9: (events/day, bytes/day), `None` for kinds the table does not list.
    pub fn catalog(self) -> Option<(f64, f64)> {
        match self {
            Self::PlayStartEnd => Some((240.0, 14.0 * KB)),
            Self::VolumeMuteSpeed => Some((60.0, 4.0 * KB)),
            Self::SeekReplay => Some((60.0, 4.0 * KB)),
            Self::QueueLibrary => Some((40.0, 2.5 * KB)),
            Self::Search => Some((20.0, 1.5 * KB)),
            Self::OutputEqWindow => Some((30.0, 2.0 * KB)),
            Self::NotInCatalog => None,
        }
    }

    /// How the group's row starts in the table of section 9 of the catalog.
    pub const fn catalog_row_prefix(self) -> Option<&'static str> {
        match self {
            Self::PlayStartEnd => Some("| Play start/end"),
            Self::VolumeMuteSpeed => Some("| Volume, mute, speed"),
            Self::SeekReplay => Some("| Seeks, replays"),
            Self::QueueLibrary => Some("| Queue, favorites"),
            Self::Search => Some("| Search"),
            Self::OutputEqWindow => Some("| Output/EQ/window"),
            Self::NotInCatalog => None,
        }
    }

    pub fn of(kind: SignalKind) -> Self {
        use SignalKind as K;
        match kind {
            K::PlayStarted | K::Skipped | K::Completed => Self::PlayStartEnd,
            K::VolumeChanged | K::SystemVolumeChanged | K::MuteToggled | K::RateChanged => {
                Self::VolumeMuteSpeed
            }
            K::SeekForward
            | K::SeekBack
            | K::Replayed
            | K::PreviousPressed
            | K::RepeatModeChanged
            | K::Paused
            | K::Resumed => Self::SeekReplay,
            K::QueuedNext
            | K::QueuedLater
            | K::RemovedFromQueue
            | K::QueueReordered
            | K::Favorited
            | K::Unfavorited
            | K::Rated
            | K::PlaylistAdded
            | K::PlaylistRemoved
            | K::MetadataEdited
            | K::ArtworkChanged => Self::QueueLibrary,
            K::SearchPerformed | K::SearchResultOpened | K::SearchNoResult => Self::Search,
            K::WindowState
            | K::OutputDeviceChanged
            | K::EqChanged
            | K::ExclusiveModeToggled
            | K::CrossfadeChanged => Self::OutputEqWindow,
            _ => Self::NotInCatalog,
        }
    }
}

/// Catalog section 9 (`docs/guides/signal-catalog.md`, the rows below "Output/EQ/window state
/// changes") for the rows that are tables rather than event groups: (rows/day, bytes/day).
/// Context snapshots: "~60", "~4.5 KB" (~70 B per row incl. the hash, its UNIQUE index and key).
pub const CATALOG_CONTEXT: (f64, f64) = (60.0, 4.5 * KB);
/// UI counters: "~300 rows", "~7 KB".
pub const CATALOG_COUNTERS: (f64, f64) = (300.0, 7.0 * KB);
/// Dictionaries and sessions: "~20 KB while the library is being discovered, saturating as it is
/// covered". The 90-day run is the discovery phase, so the 90-day average is compared with it.
pub const CATALOG_DICTIONARIES_BYTES: f64 = 20.0 * KB;

/// The tables the catalog's "Dictionaries and sessions" row covers.
const DICTIONARY_TABLES: [&str; 5] = [
    "dict_device",
    "dict_recording",
    "dict_source",
    "dict_entity",
    "session",
];

#[derive(Debug, Clone)]
pub struct GroupMeasure {
    pub group: Group,
    pub events: u64,
    /// Bytes on disk, scaled so the groups add up to the table.
    pub bytes: f64,
}

#[derive(Debug, Clone)]
pub struct KindMeasure {
    pub kind: SignalKind,
    pub events: u64,
    pub bytes: f64,
}

/// Everything measured on a finished run.
#[derive(Debug, Clone)]
pub struct Report {
    pub days: u32,
    pub page_size: u64,
    pub file_bytes: u64,
    /// Bytes per table (its indexes included), by table name.
    pub tables: BTreeMap<String, u64>,
    pub groups: Vec<GroupMeasure>,
    pub kinds: Vec<KindMeasure>,
    pub events: u64,
    pub counter_rows: u64,
    pub counter_bytes: u64,
    pub context_rows: u64,
    pub context_bytes: u64,
    /// Row-size to page-size scale factor of `signal_event`.
    pub event_scale: f64,
    pub size_samples: Vec<(u32, u64)>,
}

fn int_bytes(v: i64) -> u64 {
    match v {
        0 | 1 => 0,
        -128..=127 => 1,
        -32_768..=32_767 => 2,
        -8_388_608..=8_388_607 => 3,
        -2_147_483_648..=2_147_483_647 => 4,
        -140_737_488_355_328..=140_737_488_355_327 => 6,
        _ => 8,
    }
}

fn varint_len(v: u64) -> u64 {
    match v {
        0..=127 => 1,
        128..=16_383 => 2,
        _ => 3,
    }
}

/// Bytes a `signal_event` row takes in a leaf page: cell pointer, payload-size varint, record
/// header and body.
fn event_row_bytes(row: &rusqlite::Row<'_>) -> u64 {
    // device, seq, session, ts_off, kind, recording, source, pos_ds, payload, ctx
    let mut header = 1u64; // the header-size varint itself (the header is shorter than 128 bytes)
    let mut body = 0u64;
    for i in 0..8 {
        header += 1;
        if let Some(v) = row.get::<_, Option<i64>>(i).unwrap() {
            body += int_bytes(v);
        }
    }
    let payload = row.get::<_, Option<Vec<u8>>>(8).unwrap();
    header += 1; // blob serial type: 12 + 2n, one byte up to 57 bytes
    if let Some(p) = &payload {
        assert!(p.len() <= 57);
        body += p.len() as u64;
    }
    header += 1;
    if let Some(v) = row.get::<_, Option<i64>>(9).unwrap() {
        body += int_bytes(v);
    }
    let total = header + body;
    2 + varint_len(total) + total
}

pub fn measure(run: &Run) -> Report {
    let conn = Connection::open(&run.db.path).unwrap();
    let page_size: i64 = conn
        .query_row("PRAGMA page_size", [], |r| r.get(0))
        .unwrap();
    let page_count: i64 = conn
        .query_row("PRAGMA page_count", [], |r| r.get(0))
        .unwrap();
    let free: i64 = conn
        .query_row("PRAGMA freelist_count", [], |r| r.get(0))
        .unwrap();
    let page_size = u64::try_from(page_size).unwrap();
    let file_bytes = u64::try_from(page_count - free).unwrap() * page_size;

    let mut tables = BTreeMap::new();
    {
        let mut stmt = conn
            .prepare(
                "SELECT m.tbl_name, count(*) FROM dbstat d JOIN sqlite_master m ON m.name = d.name \
                 GROUP BY m.tbl_name",
            )
            .unwrap();
        for row in stmt
            .query_map([], |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)? as u64))
            })
            .unwrap()
        {
            let (name, pages) = row.unwrap();
            tables.insert(name, pages * page_size);
        }
    }
    let table_bytes = |name: &str| tables.get(name).copied().unwrap_or(0);

    // Attribute signal_event bytes to kinds.
    let mut per_kind: BTreeMap<u8, (u64, u64)> = BTreeMap::new();
    let mut total_estimate = 0u64;
    {
        let mut stmt = conn
            .prepare(
                "SELECT device, seq, session, ts_off, kind, recording, source, pos_ds, payload, ctx \
                 FROM signal_event",
            )
            .unwrap();
        let mut rows = stmt.query([]).unwrap();
        while let Some(row) = rows.next().unwrap() {
            let kind = row.get::<_, u8>(4).unwrap();
            let bytes = event_row_bytes(row);
            let entry = per_kind.entry(kind).or_default();
            entry.0 += 1;
            entry.1 += bytes;
            total_estimate += bytes;
        }
    }
    let event_table = table_bytes("signal_event") as f64;
    let scale = event_table / total_estimate.max(1) as f64;

    let mut kinds = Vec::new();
    let mut groups: BTreeMap<Group, (u64, f64)> = BTreeMap::new();
    let mut events = 0;
    for (code, (n, estimate)) in per_kind {
        let kind = SignalKind::from_discriminant(code).unwrap();
        let bytes = estimate as f64 * scale;
        kinds.push(KindMeasure {
            kind,
            events: n,
            bytes,
        });
        let g = groups.entry(Group::of(kind)).or_default();
        g.0 += n;
        g.1 += bytes;
        events += n;
    }
    let groups = Group::ALL
        .iter()
        .map(|&group| {
            let (events, bytes) = groups.get(&group).copied().unwrap_or_default();
            GroupMeasure {
                group,
                events,
                bytes,
            }
        })
        .collect();

    let count =
        |sql: &str| -> u64 { conn.query_row(sql, [], |r| r.get::<_, i64>(0)).unwrap() as u64 };
    Report {
        days: run.config.days,
        page_size,
        file_bytes,
        counter_rows: count("SELECT count(*) FROM ui_counter"),
        counter_bytes: table_bytes("ui_counter"),
        context_rows: count("SELECT count(*) FROM context_snapshot"),
        context_bytes: table_bytes("context_snapshot"),
        tables,
        groups,
        kinds,
        events,
        event_scale: scale,
        size_samples: run.samples.iter().map(|s| (s.day, s.file_bytes)).collect(),
    }
}

impl Report {
    /// Bytes of the dictionary tables and the session table.
    pub fn dictionary_bytes(&self) -> u64 {
        DICTIONARY_TABLES
            .iter()
            .map(|t| self.tables.get(*t).copied().unwrap_or(0))
            .sum()
    }

    fn per_day(&self, v: f64) -> f64 {
        v / f64::from(self.days)
    }

    /// Extrapolated size at 5 years with no compaction: linear growth of the hot tier.
    pub fn five_years_uncompacted_mb(&self) -> f64 {
        self.file_bytes as f64 / f64::from(self.days) * 1825.0 / 1e6
    }

    /// Extrapolated size at 5 years with the tiering of ADR 0002: the hot window stays at this
    /// size (compaction deletes older raw rows), plus the catalog's warm and cold tiers.
    ///
    /// Caveat: the 90-day figure includes the dictionary tables (1.8 MB here). Compaction never
    /// deletes dictionary rows, so they keep growing until the 50 000-track library has been
    /// covered: a few MB more than this adds up to, far under the 60 MB budget.
    pub fn five_years_with_tiering_mb(&self) -> f64 {
        let hot = self.file_bytes as f64 / f64::from(self.days) * 90.0 / 1e6;
        hot + 5.0 * (CATALOG_WARM_MB_PER_YEAR + CATALOG_COLD_MB_PER_YEAR)
    }

    /// The budgets of ADR 0002 that v1 can check, as readable violations (empty when they hold):
    /// the 90-day hot window and the 5-year extrapolation.
    pub fn budget_violations(&self) -> Vec<String> {
        let mut out = Vec::new();
        let hot_mb = self.file_bytes as f64 / f64::from(self.days) * 90.0 / 1e6;
        if hot_mb > 6.0 {
            out.push(format!(
                "90-day hot window is {hot_mb:.2} MB, over the 6 MB budget"
            ));
        }
        let five = self.five_years_with_tiering_mb();
        if five > 60.0 {
            out.push(format!(
                "5-year extrapolation with tiering is {five:.1} MB, over the 60 MB signals.db budget"
            ));
        }
        out
    }

    /// The catalog section 9 comparison for the event groups and the UI counters, as readable
    /// violations: the generator must produce the catalog's rates (+-25 %), and every row's
    /// bytes/day must stay within +50 % of the catalog.
    pub fn catalog_violations(&self) -> Vec<String> {
        let mut out = Vec::new();
        for g in &self.groups {
            let Some((cat_events, cat_bytes)) = g.group.catalog() else {
                continue;
            };
            let events_day = self.per_day(g.events as f64);
            let bytes_day = self.per_day(g.bytes);
            if events_day < cat_events * 0.75 || events_day > cat_events * 1.25 {
                out.push(format!(
                    "generator: {} produced {events_day:.0} events/day, catalog says {cat_events:.0} (+-25 % allowed)",
                    g.group.name()
                ));
            }
            if bytes_day > cat_bytes * TOLERANCE {
                out.push(format!(
                    "{}: {bytes_day:.0} B/day is {:.0} % of the catalog's {cat_bytes:.0} B/day (limit +50 %)",
                    g.group.name(),
                    bytes_day / cat_bytes * 100.0
                ));
            }
        }
        let ctr_day = self.per_day(self.counter_bytes as f64);
        if ctr_day > CATALOG_COUNTERS.1 * TOLERANCE {
            out.push(format!(
                "UI counters: {ctr_day:.0} B/day is {:.0} % of the catalog's {:.0} B/day (limit +50 %)",
                ctr_day / CATALOG_COUNTERS.1 * 100.0,
                CATALOG_COUNTERS.1
            ));
        }
        let rows_day = self.per_day(self.counter_rows as f64);
        if rows_day < CATALOG_COUNTERS.0 * 0.75 || rows_day > CATALOG_COUNTERS.0 * 1.25 {
            out.push(format!(
                "generator: {rows_day:.0} ui_counter rows/day, catalog says {:.0} (+-25 % allowed)",
                CATALOG_COUNTERS.0
            ));
        }
        let dict_day = self.per_day(self.dictionary_bytes() as f64);
        if dict_day > CATALOG_DICTIONARIES_BYTES * TOLERANCE {
            out.push(format!(
                "dictionaries + sessions: {dict_day:.0} B/day is {:.0} % of the catalog's {CATALOG_DICTIONARIES_BYTES:.0} B/day (limit +50 %)",
                dict_day / CATALOG_DICTIONARIES_BYTES * 100.0
            ));
        }
        out
    }

    /// The catalog section 9 comparison for the context snapshots (rate +-25 %, bytes/day within
    /// +50 %).
    pub fn context_violations(&self) -> Vec<String> {
        let mut out = Vec::new();
        let ctx_rows = self.per_day(self.context_rows as f64);
        if ctx_rows < CATALOG_CONTEXT.0 * 0.75 || ctx_rows > CATALOG_CONTEXT.0 * 1.25 {
            out.push(format!(
                "generator: {ctx_rows:.0} context snapshots/day, catalog says {:.0} (+-25 % allowed)",
                CATALOG_CONTEXT.0
            ));
        }
        let ctx_day = self.per_day(self.context_bytes as f64);
        if ctx_day > CATALOG_CONTEXT.1 * TOLERANCE {
            out.push(format!(
                "context snapshots: {ctx_day:.0} B/day is {:.0} % of the catalog's {:.0} B/day (limit +50 %)",
                ctx_day / CATALOG_CONTEXT.1 * 100.0,
                CATALOG_CONTEXT.1
            ));
        }
        out
    }

    /// The readable report: per-table sizes, per-group bytes/day against the catalog, the
    /// per-kind detail and the extrapolation.
    pub fn render(&self, run: &Run) -> String {
        let d = f64::from(self.days);
        let mut s = String::new();
        let _ = writeln!(
            s,
            "\nSTORAGE GROWTH: {} simulated days, seed {}, {} plays ({:.0}/day), {} events ({:.0}/day), run time {:.1} s ({:.1} s of it waiting for the writer)",
            self.days,
            run.config.seed,
            run.plays,
            run.plays as f64 / d,
            self.events,
            self.events as f64 / d,
            run.elapsed.as_secs_f64(),
            run.barrier_wait.as_secs_f64()
        );
        let _ = writeln!(
            s,
            "writer: {} transactions, {} failed, {} events lost, {} rejected; bus: {} dropped, {} reserved overflows, {} counter keys dropped",
            run.writer.batches_committed,
            run.writer.failed_commits,
            run.writer.events_lost,
            run.writer.events_rejected,
            run.dropped,
            run.reserved_overflows,
            run.keys_dropped
        );
        let _ = writeln!(
            s,
            "coalescing: {} volume ticks, {} system-volume ticks, {} rate ticks, {} seek ticks fed -> {} gestures ended without an event; {} counter interactions -> {} ui_counter rows",
            run.ticks.volume,
            run.ticks.system_volume,
            run.ticks.rate,
            run.ticks.seek,
            run.suppressed_gestures,
            run.ticks.counter_interactions,
            self.counter_rows
        );
        let _ = writeln!(s, "\nPer table (pages from dbstat, indexes included):");
        for (name, bytes) in &self.tables {
            let _ = writeln!(
                s,
                "  {name:<20} {:>10} B  {:>7.1} KB",
                bytes,
                *bytes as f64 / 1000.0
            );
        }
        let _ = writeln!(
            s,
            "  {:<20} {:>10} B  ({:.2} MB file, {:.1} B/event whole file)",
            "FILE",
            self.file_bytes,
            self.file_bytes as f64 / 1e6,
            self.file_bytes as f64 / self.events.max(1) as f64
        );
        let _ = writeln!(
            s,
            "\nPer signal group, per day, against catalog section 9 (1 KB = 1000 B; row sizes from the file format, scaled x{:.3} to the table's pages):",
            self.event_scale
        );
        let _ = writeln!(
            s,
            "  {:<46} {:>9} {:>9} {:>10} {:>10} {:>7}",
            "group", "ev/day", "cat ev", "B/day", "cat B/day", "ratio"
        );
        let mut total_bytes = 0.0;
        let mut total_catalog = 0.0;
        for g in &self.groups {
            let (cat_ev, cat_b) = g
                .group
                .catalog()
                .map_or(("-".to_owned(), "-".to_owned()), |(e, b)| {
                    (format!("{e:.0}"), format!("{b:.0}"))
                });
            let ratio = g.group.catalog().map_or("-".to_owned(), |(_, b)| {
                format!("{:.2}", self.per_day(g.bytes) / b)
            });
            if let Some((_, b)) = g.group.catalog() {
                total_catalog += b;
            }
            total_bytes += self.per_day(g.bytes);
            let _ = writeln!(
                s,
                "  {:<46} {:>9.1} {:>9} {:>10.0} {:>10} {:>7}",
                g.group.name(),
                self.per_day(g.events as f64),
                cat_ev,
                self.per_day(g.bytes),
                cat_b,
                ratio
            );
        }
        let rows = [
            (
                "Context snapshots (context_snapshot)",
                self.context_rows,
                self.context_bytes,
                CATALOG_CONTEXT,
            ),
            (
                "UI counters (ui_counter rows)",
                self.counter_rows,
                self.counter_bytes,
                CATALOG_COUNTERS,
            ),
        ];
        for (name, n, bytes, (cat_n, cat_b)) in rows {
            total_bytes += self.per_day(bytes as f64);
            total_catalog += cat_b;
            let _ = writeln!(
                s,
                "  {:<46} {:>9.1} {:>9.0} {:>10.0} {:>10.0} {:>7.2}",
                name,
                self.per_day(n as f64),
                cat_n,
                self.per_day(bytes as f64),
                cat_b,
                self.per_day(bytes as f64) / cat_b
            );
        }
        let _ = writeln!(
            s,
            "  {:<46} {:>9} {:>9} {:>10.0} {:>10.0}   (the catalog's total also has 6 KB/day of play records, which T-0029 builds; the first six rows are conservative upper bounds)",
            "Total of the rows above", "", "", total_bytes, total_catalog
        );
        let dictionaries = self.dictionary_bytes();
        let _ = writeln!(
            s,
            "  {:<46} {:>9} {:>9} {:>10.0} {:>10.0} {:>7.2}",
            "Dictionaries and sessions (5 tables)",
            "",
            "",
            self.per_day(dictionaries as f64),
            CATALOG_DICTIONARIES_BYTES,
            self.per_day(dictionaries as f64) / CATALOG_DICTIONARIES_BYTES
        );
        let _ = writeln!(s, "\nPer kind:");
        let _ = writeln!(
            s,
            "  {:<22} {:>8} {:>8} {:>8}",
            "kind", "ev/day", "B/event", "B/day"
        );
        for k in &self.kinds {
            let _ = writeln!(
                s,
                "  {:<22} {:>8.1} {:>8.1} {:>8.0}",
                k.kind.name(),
                self.per_day(k.events as f64),
                k.bytes / k.events as f64,
                self.per_day(k.bytes)
            );
        }
        let _ = writeln!(s, "\nGrowth (file size, no compaction):");
        for (day, bytes) in &self.size_samples {
            let _ = writeln!(s, "  day {day:>3}: {:.2} MB", *bytes as f64 / 1e6);
        }
        let _ = writeln!(
            s,
            "\nExtrapolation: {:.1} KB/day. 90-day hot window {:.2} MB (budget 6 MB). 5 years without compaction {:.0} MB (why compaction exists). 5 years with ADR 0002 tiering (hot window + catalog warm {:.1} + cold {:.1} MB/year; dictionaries are not compacted and keep growing until the library is covered, a few MB more) {:.1} MB (budget 60 MB).",
            self.file_bytes as f64 / d / 1000.0,
            self.file_bytes as f64 / d * 90.0 / 1e6,
            self.five_years_uncompacted_mb(),
            CATALOG_WARM_MB_PER_YEAR,
            CATALOG_COLD_MB_PER_YEAR,
            self.five_years_with_tiering_mb()
        );
        s
    }
}
