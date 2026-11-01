//! In-memory aggregation of Counter-class signals (`Impression`, `ArtworkHover`, `PageDwell`).
//!
//! `docs/rules/storage-budget.md` section 1: a signal expected to fire more than ~50 times per
//! hour is a counter, not an event. The UI calls [`UiCounters`] on every impression; the
//! writer drains the aggregated rows every few seconds and **adds** them to the `ui_counter`
//! table (`docs/rules/database.md`), keyed `(day, kind, entity_kind, entity)`.
//!
//! # Assumptions
//!
//! - `day` is the *local* calendar day as `yyyymmdd`, computed from the injected clock plus the
//!   timezone offset the bus was given ([`UiCounters::set_tz_offset`]). The bus has no session
//!   of its own, so whoever starts a session (and therefore knows its offset) calls the setter;
//!   until then the offset is the one in the bus configuration.
//! - The entity is a [`CounterEntity`]: an [`EntityKind`] and the entity's 16-byte UUID, which is
//!   all a UI producer has. This crate never sees a dictionary id: the writer (T-0006) maps
//!   `(entity_kind, uuid)` to the small integer `ui_counter.entity` when it flushes (`dict_recording`
//!   for [`EntityKind::Track`], `dict_entity` for the rest; `docs/rules/database.md`,
//!   `docs/rules/storage-budget.md` section 3), inside the same transaction as the rows.
//! - A counter row is *added* to the stored row on flush, not replaced: the writer flushes many
//!   times a day, so [`CounterRow::count`] and [`CounterRow::total_ms`] are deltas since the
//!   previous drain and `max_scrolled` is a running maximum.

use std::collections::HashMap;
use std::sync::atomic::{AtomicI16, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use tinwave_core::{Clock, RecordingId, TzOffsetMin, UtcMillis};

use crate::{EntityKind, Fraction, SignalKind, SignalSettings};

/// A local calendar day as `yyyymmdd` (the `ui_counter.day` column), e.g. `20261102`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct LocalDay(u32);

impl LocalDay {
    /// The local day containing `ts` at the given timezone offset.
    ///
    /// Years are clamped to `0..=9999` so the value always fits `yyyymmdd`; a clock that far
    /// off is already wrong everywhere else, and a counter key must not fail.
    #[must_use]
    pub fn from_utc(ts: UtcMillis, tz: TzOffsetMin) -> Self {
        const MS_PER_DAY: i64 = 86_400_000;
        let local_ms = ts.as_i64().saturating_add(i64::from(tz.minutes()) * 60_000);
        let days = local_ms.div_euclid(MS_PER_DAY);
        // Howard Hinnant's `civil_from_days`: days since 1970-01-01 to (year, month, day).
        let z = days.saturating_add(719_468);
        let era = z.div_euclid(146_097);
        let doe = z.rem_euclid(146_097);
        let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
        let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
        let mp = (5 * doy + 2) / 153;
        let day = doy - (153 * mp + 2) / 5 + 1;
        let month = if mp < 10 { mp + 3 } else { mp - 9 };
        let year = (yoe + era.saturating_mul(400) + i64::from(month <= 2)).clamp(0, 9_999);
        // year <= 9999, month <= 12, day <= 31: the sum is below 100_000_000.
        Self(u32::try_from(year * 10_000 + month * 100 + day).unwrap_or(99_991_231))
    }

    /// The `yyyymmdd` value stored in the database.
    #[must_use]
    pub const fn yyyymmdd(self) -> u32 {
        self.0
    }
}

/// What a counter row is about: the kind of entity and its UUID.
///
/// This is what the UI knows. Dictionary ids are the writer's business: `kind` selects the
/// dictionary (`EntityKind::Track` is a recording UUID, resolved through `dict_recording`; every
/// other kind through `dict_entity`), so two entities of different kinds never collide.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct CounterEntity {
    /// Track, album, artist or playlist.
    pub kind: EntityKind,
    /// The entity's UUID (16 raw bytes, as stored in the dictionaries).
    pub uuid: [u8; 16],
}

impl CounterEntity {
    /// An entity of `kind` with the given UUID bytes.
    #[must_use]
    pub const fn new(kind: EntityKind, uuid: [u8; 16]) -> Self {
        Self { kind, uuid }
    }

    /// A track: the UUID is the recording's.
    #[must_use]
    pub const fn track(recording: RecordingId) -> Self {
        Self::new(EntityKind::Track, *recording.as_bytes())
    }
}

/// One aggregated `ui_counter` row, as handed to the sink.
///
/// The sink resolves `(entity_kind, entity)` to a dictionary id and adds the deltas to the row
/// `(day, kind, dictionary entity_kind, dictionary id)`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CounterRow {
    /// Local day the interactions happened on.
    pub day: LocalDay,
    /// `Impression`, `ArtworkHover` or `PageDwell`.
    pub kind: SignalKind,
    /// Kind of the entity the interactions were about; selects the dictionary
    /// (`dict_recording` for `Track`, `dict_entity` otherwise).
    pub entity_kind: EntityKind,
    /// UUID of that entity (16 raw bytes).
    pub entity: [u8; 16],
    /// Interactions since the previous drain; the sink adds this to the stored count.
    pub count: u64,
    /// Total milliseconds (hover or dwell time) since the previous drain; 0 for impressions.
    pub total_ms: u64,
    /// Deepest scroll seen (`PageDwell` only, 0 otherwise); the sink keeps the maximum.
    pub max_scrolled: Fraction,
}

/// What [`UiCounters`] did with an interaction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[must_use]
pub enum CountOutcome {
    /// Added to its counter.
    Counted,
    /// The category or tier is off; nothing was recorded.
    Disabled,
    /// The key table is full and this `(day, kind, entity_kind, entity)` is new; the interaction was lost
    /// and counted in [`UiCounters::keys_dropped`]. Existing keys keep counting.
    KeysFull,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct CounterKey {
    day: LocalDay,
    kind: SignalKind,
    entity: CounterEntity,
}

type CounterMap = HashMap<CounterKey, Totals>;

#[derive(Debug, Default, Clone, Copy)]
struct Totals {
    count: u64,
    total_ms: u64,
    max_scrolled: u8,
}

/// Aggregates `Impression`, `ArtworkHover` and `PageDwell` interactions per
/// `(day, kind, entity_kind, entity)`. Cheap and thread-safe: one short mutex hold per interaction.
pub struct UiCounters {
    clock: Arc<dyn Clock>,
    settings: Arc<SignalSettings>,
    tz_offset_min: AtomicI16,
    max_keys: usize,
    rows: Mutex<CounterMap>,
    /// An empty map swapped in by [`drain_into`](Self::drain_into) so that the hot mutex is held
    /// for a pointer swap only. Touched by the drain alone.
    spare: Mutex<CounterMap>,
    keys_dropped: AtomicU64,
}

impl std::fmt::Debug for UiCounters {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("UiCounters")
            .field("max_keys", &self.max_keys)
            .field("keys_dropped", &self.keys_dropped())
            .finish_non_exhaustive()
    }
}

impl UiCounters {
    /// Counters on `clock`, honouring `settings`, holding at most `max_keys` distinct rows
    /// between drains (each row is about 64 bytes and the drain keeps a spare table of the same
    /// size, so 16 384 keys is about 2 MB at most).
    #[must_use]
    pub fn new(
        clock: Arc<dyn Clock>,
        settings: Arc<SignalSettings>,
        tz_offset: TzOffsetMin,
        max_keys: usize,
    ) -> Self {
        Self {
            clock,
            settings,
            tz_offset_min: AtomicI16::new(tz_offset.minutes()),
            max_keys,
            rows: Mutex::new(HashMap::new()),
            spare: Mutex::new(HashMap::new()),
            keys_dropped: AtomicU64::new(0),
        }
    }

    /// Sets the timezone offset used to compute the local day (call when a session starts).
    pub fn set_tz_offset(&self, tz: TzOffsetMin) {
        self.tz_offset_min.store(tz.minutes(), Ordering::Relaxed);
    }

    /// An item was visible for 500 ms or more in a settled list.
    pub fn impression(&self, entity: CounterEntity) -> CountOutcome {
        self.record(SignalKind::Impression, entity, 0, Fraction::MIN)
    }

    /// The artwork was hovered for `hover_ms` (800 ms or more).
    pub fn artwork_hover(&self, entity: CounterEntity, hover_ms: u32) -> CountOutcome {
        self.record(SignalKind::ArtworkHover, entity, hover_ms, Fraction::MIN)
    }

    /// An album, artist or playlist page was left after `dwell_ms`, scrolled to `scrolled`.
    pub fn page_dwell(
        &self,
        entity: CounterEntity,
        dwell_ms: u32,
        scrolled: Fraction,
    ) -> CountOutcome {
        self.record(SignalKind::PageDwell, entity, dwell_ms, scrolled)
    }

    fn record(
        &self,
        kind: SignalKind,
        entity: CounterEntity,
        ms: u32,
        scrolled: Fraction,
    ) -> CountOutcome {
        if !self.settings.enabled(kind) {
            return CountOutcome::Disabled;
        }
        let key = CounterKey {
            day: self.today(),
            kind,
            entity,
        };
        let mut rows = self.rows.lock().unwrap_or_else(PoisonError::into_inner);
        let at_capacity = rows.len() >= self.max_keys;
        if at_capacity && !rows.contains_key(&key) {
            drop(rows);
            self.keys_dropped.fetch_add(1, Ordering::Relaxed);
            return CountOutcome::KeysFull;
        }
        let totals = rows.entry(key).or_default();
        totals.count = totals.count.saturating_add(1);
        totals.total_ms = totals.total_ms.saturating_add(u64::from(ms));
        totals.max_scrolled = totals.max_scrolled.max(scrolled.raw());
        CountOutcome::Counted
    }

    fn today(&self) -> LocalDay {
        let minutes = self.tz_offset_min.load(Ordering::Relaxed);
        // Only ever stored from a validated `TzOffsetMin`; UTC is the fallback for a
        // corrupted value, which cannot occur.
        let tz = TzOffsetMin::new(minutes).unwrap_or(TzOffsetMin::UTC);
        LocalDay::from_utc(self.clock.now(), tz)
    }

    /// Moves every aggregated row into `out`, ordered by `(day, kind, entity_kind, entity)`, and
    /// resets the counters. Returns the number of rows appended.
    ///
    /// The mutex the UI threads count under is held for one pointer swap, not for building up to
    /// `max_keys` rows: the full table is swapped for an empty spare and emptied afterwards, outside
    /// the lock. Both tables keep their allocation.
    pub fn drain_into(&self, out: &mut Vec<CounterRow>) -> usize {
        let start = out.len();
        // Held for the whole drain so two drains cannot fight over the spare; producers never take it.
        let mut spare = self.spare.lock().unwrap_or_else(PoisonError::into_inner);
        {
            let mut rows = self.rows.lock().unwrap_or_else(PoisonError::into_inner);
            std::mem::swap(&mut *rows, &mut *spare);
        }
        out.extend(spare.drain().map(|(key, totals)| CounterRow {
            day: key.day,
            kind: key.kind,
            entity_kind: key.entity.kind,
            entity: key.entity.uuid,
            count: totals.count,
            total_ms: totals.total_ms,
            max_scrolled: Fraction::from_raw(totals.max_scrolled),
        }));
        out[start..].sort_unstable_by_key(|r| (r.day, r.kind, r.entity_kind.code(), r.entity));
        out.len() - start
    }

    /// Distinct rows currently held.
    #[must_use]
    pub fn len(&self) -> usize {
        self.rows
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .len()
    }

    /// Whether no interaction has been counted since the last drain.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Interactions lost because the key table was full (diagnostics, never persisted).
    #[must_use]
    pub fn keys_dropped(&self) -> u64 {
        self.keys_dropped.load(Ordering::Relaxed)
    }
}

#[cfg(test)]
mod tests {
    use tinwave_core::{ManualClock, Tier};

    use super::*;
    use crate::SignalCategory;

    const NOV_2_2026: i64 = 1_793_577_600_000;

    fn counters(clock: &ManualClock) -> (UiCounters, Arc<SignalSettings>) {
        let settings = Arc::new(SignalSettings::new());
        let c = UiCounters::new(
            Arc::new(clock.clone()),
            settings.clone(),
            TzOffsetMin::UTC,
            1_000,
        );
        (c, settings)
    }

    /// An entity of the given kind whose UUID is `n` repeated.
    fn of(kind: EntityKind, n: u8) -> CounterEntity {
        CounterEntity::new(kind, [n; 16])
    }

    fn ent(n: u8) -> CounterEntity {
        of(EntityKind::Track, n)
    }

    fn drain(c: &UiCounters) -> Vec<CounterRow> {
        let mut out = Vec::new();
        c.drain_into(&mut out);
        out
    }

    #[test]
    fn local_day_matches_known_calendar_dates() {
        let utc = TzOffsetMin::UTC;
        let day = |ms, tz| LocalDay::from_utc(UtcMillis::new(ms), tz).yyyymmdd();
        assert_eq!(day(0, utc), 19_700_101);
        assert_eq!(day(-1_000, utc), 19_691_231);
        assert_eq!(day(NOV_2_2026, utc), 20_261_102);
        assert_eq!(day(1_709_164_800_000, utc), 20_240_229, "leap day");
        assert_eq!(day(951_868_800_000, utc), 20_000_301, "2000 is a leap year");
        assert_eq!(day(1_798_759_800_000, utc), 20_261_231);
    }

    #[test]
    fn local_day_applies_the_timezone_offset() {
        let late = UtcMillis::new(1_798_759_800_000); // 2026-12-31 23:30 UTC
        let east = TzOffsetMin::new(60).unwrap();
        let west = TzOffsetMin::new(-60).unwrap();
        let far_east = TzOffsetMin::new(840).unwrap();
        assert_eq!(LocalDay::from_utc(late, east).yyyymmdd(), 20_270_101);
        assert_eq!(LocalDay::from_utc(late, west).yyyymmdd(), 20_261_231);
        assert_eq!(LocalDay::from_utc(late, far_east).yyyymmdd(), 20_270_101);
        let early = UtcMillis::new(NOV_2_2026 + 30 * 60_000); // 00:30 UTC
        assert_eq!(LocalDay::from_utc(early, west).yyyymmdd(), 20_261_101);
    }

    #[test]
    fn local_day_survives_extreme_clocks() {
        let tz = TzOffsetMin::UTC;
        assert_eq!(
            LocalDay::from_utc(UtcMillis::new(i64::MAX), tz).yyyymmdd() / 10_000,
            9_999
        );
        assert_eq!(
            LocalDay::from_utc(UtcMillis::new(i64::MIN), tz).yyyymmdd() / 10_000,
            0
        );
    }

    #[test]
    fn a_thousand_impressions_on_one_entity_are_one_row_with_count_1000() {
        let clock = ManualClock::new(UtcMillis::new(NOV_2_2026));
        let (c, _) = counters(&clock);
        for _ in 0..1_000 {
            assert_eq!(c.impression(ent(7)), CountOutcome::Counted);
        }
        assert_eq!(c.len(), 1);
        let rows = drain(&c);
        assert_eq!(
            rows,
            [CounterRow {
                day: LocalDay(20_261_102),
                kind: SignalKind::Impression,
                entity_kind: EntityKind::Track,
                entity: [7; 16],
                count: 1_000,
                total_ms: 0,
                max_scrolled: Fraction::MIN,
            }]
        );
        assert!(c.is_empty(), "drained");
        assert!(drain(&c).is_empty());
    }

    #[test]
    fn hover_and_dwell_accumulate_time_and_keep_the_deepest_scroll() {
        let clock = ManualClock::new(UtcMillis::new(NOV_2_2026));
        let (c, _) = counters(&clock);
        let e = ent(1);
        let _ = c.artwork_hover(e, 900);
        let _ = c.artwork_hover(e, 1_100);
        let _ = c.page_dwell(e, 5_000, Fraction::from_raw(100));
        let _ = c.page_dwell(e, 2_000, Fraction::from_raw(200));
        let _ = c.page_dwell(e, 1_000, Fraction::from_raw(50));
        let rows = drain(&c);
        assert_eq!(rows.len(), 2, "one row per kind");
        let hover = rows.iter().find(|r| r.kind == SignalKind::ArtworkHover);
        let dwell = rows.iter().find(|r| r.kind == SignalKind::PageDwell);
        let hover = hover.unwrap();
        assert_eq!((hover.count, hover.total_ms), (2, 2_000));
        let dwell = dwell.unwrap();
        assert_eq!((dwell.count, dwell.total_ms), (3, 8_000));
        assert_eq!(dwell.max_scrolled.raw(), 200);
    }

    #[test]
    fn rows_are_keyed_by_entity_kind_and_day() {
        let clock = ManualClock::new(UtcMillis::new(NOV_2_2026));
        let (c, _) = counters(&clock);
        let _ = c.impression(ent(2));
        let _ = c.impression(ent(1));
        let _ = c.artwork_hover(ent(1), 900);
        clock.advance(86_400_000);
        let _ = c.impression(ent(1));
        let rows = drain(&c);
        let keys: Vec<_> = rows
            .iter()
            .map(|r| (r.day.yyyymmdd(), r.kind, r.entity[0]))
            .collect();
        assert_eq!(
            keys,
            [
                (20_261_102, SignalKind::ArtworkHover, 1),
                (20_261_102, SignalKind::Impression, 1),
                (20_261_102, SignalKind::Impression, 2),
                (20_261_103, SignalKind::Impression, 1),
            ]
        );
    }

    #[test]
    fn the_same_uuid_under_another_entity_kind_is_another_row() {
        let clock = ManualClock::new(UtcMillis::new(NOV_2_2026));
        let (c, _) = counters(&clock);
        let _ = c.impression(of(EntityKind::Playlist, 5));
        let _ = c.impression(of(EntityKind::Track, 5));
        let _ = c.impression(of(EntityKind::Album, 5));
        let _ = c.impression(of(EntityKind::Album, 5));
        let _ = c.impression(of(EntityKind::Artist, 5));
        let rows = drain(&c);
        let got: Vec<_> = rows
            .iter()
            .map(|r| (r.entity_kind, r.entity, r.count))
            .collect();
        assert_eq!(
            got,
            [
                (EntityKind::Track, [5; 16], 1),
                (EntityKind::Album, [5; 16], 2),
                (EntityKind::Artist, [5; 16], 1),
                (EntityKind::Playlist, [5; 16], 1),
            ],
            "one row per (entity_kind, uuid), ordered by entity kind code"
        );
    }

    #[test]
    fn a_track_counter_carries_the_recording_uuid_bytes() {
        let id = RecordingId::from_parts(UtcMillis::new(NOV_2_2026), [4; 10]).unwrap();
        let entity = CounterEntity::track(id);
        assert_eq!(entity.kind, EntityKind::Track);
        assert_eq!(&entity.uuid, id.as_bytes());
        let clock = ManualClock::new(UtcMillis::new(NOV_2_2026));
        let (c, _) = counters(&clock);
        let _ = c.impression(entity);
        let rows = drain(&c);
        assert_eq!(
            (rows[0].entity_kind, &rows[0].entity),
            (EntityKind::Track, id.as_bytes())
        );
    }

    #[test]
    fn counting_continues_across_drains_and_reuses_the_swapped_tables() {
        let clock = ManualClock::new(UtcMillis::new(NOV_2_2026));
        let (c, _) = counters(&clock);
        for round in 1..=3_u64 {
            for n in 0..50_u8 {
                for _ in 0..round {
                    let _ = c.impression(ent(n));
                }
            }
            assert_eq!(c.len(), 50);
            let rows = drain(&c);
            assert_eq!(rows.len(), 50);
            assert!(rows.iter().all(|r| r.count == round), "deltas, not totals");
            assert!(c.is_empty());
            assert!(drain(&c).is_empty());
        }
    }

    #[test]
    fn no_interaction_is_lost_or_doubled_when_draining_while_counting() {
        let clock = ManualClock::new(UtcMillis::new(NOV_2_2026));
        let (c, _) = counters(&clock);
        let c = Arc::new(c);
        let per_thread = 5_000_u64;
        let handles: Vec<_> = (0..3)
            .map(|_| {
                let c = c.clone();
                std::thread::spawn(move || {
                    for _ in 0..per_thread {
                        let _ = c.impression(ent(1));
                    }
                })
            })
            .collect();
        let mut seen = 0;
        while !handles.iter().all(std::thread::JoinHandle::is_finished) {
            seen += drain(&c).iter().map(|r| r.count).sum::<u64>();
        }
        for h in handles {
            h.join().unwrap();
        }
        seen += drain(&c).iter().map(|r| r.count).sum::<u64>();
        assert_eq!(seen, 3 * per_thread);
    }

    #[test]
    fn the_day_rolls_over_at_local_midnight() {
        let clock = ManualClock::new(UtcMillis::new(NOV_2_2026 + 23 * 3_600_000)); // 23:00 UTC
        let (c, _) = counters(&clock);
        c.set_tz_offset(TzOffsetMin::new(120).unwrap()); // local 01:00 on the 3rd
        let _ = c.impression(ent(1));
        assert_eq!(drain(&c)[0].day.yyyymmdd(), 20_261_103);
    }

    #[test]
    fn disabled_category_or_tier_records_nothing() {
        let clock = ManualClock::new(UtcMillis::new(NOV_2_2026));
        let (c, settings) = counters(&clock);
        settings.set_category(SignalCategory::Browsing, false);
        assert_eq!(c.impression(ent(1)), CountOutcome::Disabled);
        assert_eq!(c.artwork_hover(ent(1), 900), CountOutcome::Disabled);
        assert_eq!(
            c.page_dwell(ent(1), 900, Fraction::MAX),
            CountOutcome::Disabled
        );
        assert!(c.is_empty());
        settings.set_category(SignalCategory::Browsing, true);
        settings.set_tier(Tier::T0, false);
        assert_eq!(c.impression(ent(1)), CountOutcome::Disabled);
        assert!(c.is_empty());
    }

    #[test]
    fn a_full_table_drops_new_keys_but_keeps_counting_existing_ones() {
        let clock = ManualClock::new(UtcMillis::new(NOV_2_2026));
        let c = UiCounters::new(
            Arc::new(clock),
            Arc::new(SignalSettings::new()),
            TzOffsetMin::UTC,
            2,
        );
        assert_eq!(c.impression(ent(1)), CountOutcome::Counted);
        assert_eq!(c.impression(ent(2)), CountOutcome::Counted);
        assert_eq!(c.impression(ent(3)), CountOutcome::KeysFull);
        assert_eq!(c.impression(ent(1)), CountOutcome::Counted);
        assert_eq!(c.keys_dropped(), 1);
        let rows = drain(&c);
        assert_eq!(rows.iter().map(|r| r.count).sum::<u64>(), 3);
        // Draining frees the table.
        assert_eq!(c.impression(ent(3)), CountOutcome::Counted);
    }

    #[test]
    fn counting_is_thread_safe() {
        let clock = ManualClock::new(UtcMillis::new(NOV_2_2026));
        let (c, _) = counters(&clock);
        let c = Arc::new(c);
        let handles: Vec<_> = (0..4)
            .map(|_| {
                let c = c.clone();
                std::thread::spawn(move || {
                    for _ in 0..500 {
                        let _ = c.impression(ent(9));
                    }
                })
            })
            .collect();
        for h in handles {
            h.join().unwrap();
        }
        let rows = drain(&c);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].count, 2_000);
    }
}
