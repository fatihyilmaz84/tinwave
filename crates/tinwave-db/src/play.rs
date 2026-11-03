//! Play records: one `play` row per play, written in the transaction of the batch that holds the
//! play's outcome (`docs/rules/database.md`, "Write path").
//!
//! # Which events make a row
//!
//! The signal stream describes a play with up to two events: `PlayStarted` when the listener first
//! heard the track, and `Completed` or `Skipped` when it was left. A row is written when an
//! outcome event is stored, from the play's events:
//!
//! | column | source |
//! |---|---|
//! | `device`, `session`, `recording`, `source` | the `PlayStarted` row (the session the play began in) |
//! | `started_ts` | the `PlayStarted` time, UTC ms |
//! | `listened_ds` | `Completed`: its `listened_ds`. `Skipped`: the time from the start to the skip minus the time spent paused in between (from the `Paused` / `Resumed` events), so it ignores seeks and rate |
//! | `end_reason` | [`EndReason`] |
//! | `end_frac` | `Skipped`: its fraction. `Completed`: 255 (it reached the end, or the last 3 %) |
//! | `flags` | [`PlayFlags`] |
//! | `peak_section` | `NULL`: there is no audio analysis yet (E-03 fills it) |
//! | `rate` | `NULL` at first; a derived value stores it afterwards, see "Derived values" |
//! | `context_key` | the `key` of the `context_snapshot` the `PlayStarted` event points at; 0 (nothing known) when it has none, or the row or key is unusable |
//!
//! # Pairing: per device, in `seq` order, never per session
//!
//! A play's outcome can be stored in a later session than its start (a track paused for 30 minutes
//! ends the session; the skip that follows opens the next one), and batches are cut anywhere. So the
//! start of an outcome is found in the table: the nearest earlier `PlayStarted`, `Skipped` or
//! `Completed` of the **same device** by `seq`. It pairs only if that is a `PlayStarted` of the same
//! recording; if it is another outcome the play already ended, and if it is a start of another
//! recording the events do not belong together. An outcome without a start writes no row
//! ([`PlayWrite::Unpaired`], counted and logged), because a row needs the start's time and
//! recording.
//!
//! **A start pairs once.** A start whose play already has a row (same device, session, recording
//! and start time) is not paired again. Normally the nearest-boundary rule already guarantees it,
//! because the play's own outcome sits between that start and any later one. But a wipe of the
//! seek, skip and replay category deletes every stored outcome and keeps the `play` rows, so an
//! old start is the nearest boundary again: if the *new* start of the same recording was never
//! stored (the queue and library category switched off while seek stays on), its outcome would
//! pair with the old start and write a second row with an old `started_ts` and a wrong
//! `listened_ds`. Skipping a start that has a row cannot drop a legitimate pairing: a play's row is
//! written in the batch that stores its first outcome, and a second outcome of the same play has
//! the first as its nearest boundary (so it was unpaired already). It does not depend on a time
//! window, so a play paused for hours still pairs.
//!
//! # Which plays have no row
//!
//! - A track that fails to open leaves no signal at all. A track that fails after it was heard
//!   leaves a `PlayStarted` and no outcome. Neither writes a row: a decode or IO failure is not a
//!   listening choice, and counting it as a skip would teach the taste engine a false dislike.
//! - **Stopped and quit plays cannot be derived yet.** The playback controller emits no signal when
//!   a play is stopped before its last 3 % (or at quit): it is also a bare `PlayStarted`, which is
//!   indistinguishable from a failure. [`EndReason::Stopped`] and [`EndReason::Quit`] therefore
//!   exist as stable codes, but nothing writes them until a signal says which it was.
//!
//! # Derived values
//!
//! A derived signal (catalog class D) is never a `signal_event` row; its value goes into the `play`
//! row of the play it describes ([`write_derived`]). The value names its play by what the row copies
//! from the start (device, session, recording and the `PlayStarted` time), so the row is found by
//! an equality `UPDATE` on those four and the `play_rec_ts` index, and no dictionary or session row is
//! created for a play that has none: a value whose play has no row is [`DerivedWrite::NoPlay`].
//! Today that is `RateSession` into `rate` (a `u8` of hundredths minus 50, `NULL` when not measured).
//! Storing the same value twice (a retried batch) leaves the same row.
//!
//! Writing is idempotent per event: a row is derived only for an outcome event that was newly
//! inserted, so a re-sent event (sync) or a retried batch never doubles a play.

use rusqlite::{params, OptionalExtension, Transaction};
use tinwave_signals::{ContextKey, DerivedPayload, DerivedValue, PositionDs, SignalKind};

use crate::DbError;

/// Why a play ended: the `play.end_reason` codes, frozen (never renumbered or reused).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum EndReason {
    /// Reached the end, or was left with at most 3 % to go.
    Completed,
    /// Skipped with less than 30 s listened.
    SkippedEarly,
    /// Skipped after 30 s and before the last 20 % of the track.
    SkippedMid,
    /// Skipped in the last 20 % of the track (more than 80 % played).
    SkippedLate,
    /// Stopped by the listener. Not written yet (module docs).
    Stopped,
    /// The application quit during the play. Not written yet (module docs).
    Quit,
    /// How the play ended was wiped (`SignalWiper`: the seek/skip/replay category holds the
    /// outcome signals this column comes from). `listened_ds` and `end_frac` are 0 and mean
    /// nothing. Never written by the play writer; readers must treat it as no information, not as a
    /// skip.
    Unknown,
}

impl EndReason {
    /// Every reason, in code order.
    pub const ALL: [Self; 7] = [
        Self::Completed,
        Self::SkippedEarly,
        Self::SkippedMid,
        Self::SkippedLate,
        Self::Stopped,
        Self::Quit,
        Self::Unknown,
    ];

    /// A skip with less than this much listened (deciseconds) is early: the catalog's two
    /// strongest negative skip buckets are under 5 s and 5 s to 30 s.
    pub const EARLY_BELOW_DS: u32 = 300;

    /// A skip beyond this fraction of the track (per-255; 204 is 80 %) is late: the catalog
    /// scores a skip past 80 % as mildly positive.
    pub const LATE_ABOVE_RAW: u8 = 204;

    /// The stored code.
    #[must_use]
    pub const fn code(self) -> u8 {
        match self {
            Self::Completed => 0,
            Self::SkippedEarly => 1,
            Self::SkippedMid => 2,
            Self::SkippedLate => 3,
            Self::Stopped => 4,
            Self::Quit => 5,
            Self::Unknown => 6,
        }
    }

    /// The reason for a stored code.
    #[must_use]
    pub const fn from_code(code: u8) -> Option<Self> {
        match code {
            0 => Some(Self::Completed),
            1 => Some(Self::SkippedEarly),
            2 => Some(Self::SkippedMid),
            3 => Some(Self::SkippedLate),
            4 => Some(Self::Stopped),
            5 => Some(Self::Quit),
            6 => Some(Self::Unknown),
            _ => None,
        }
    }

    /// The reason for a skip after `listened_ds` deciseconds, at `fraction_raw` (per-255) of the
    /// track. Past 80 % is late whatever was listened; otherwise under 30 s is early.
    #[must_use]
    pub const fn of_skip(listened_ds: u32, fraction_raw: u8) -> Self {
        if fraction_raw > Self::LATE_ABOVE_RAW {
            Self::SkippedLate
        } else if listened_ds < Self::EARLY_BELOW_DS {
            Self::SkippedEarly
        } else {
            Self::SkippedMid
        }
    }
}

/// The `play.flags` bit set (`u16`). The bit order is the order `docs/rules/storage-budget.md`
/// lists the flags in; a bit is never moved or reused.
///
/// Only the flags whose source event exists today are set; the other documented flags
/// (`seek_fwd_intro` needs sections, the volume and rate flags need payload analysis, `lyrics`,
/// `queued` and `removed_after` have no source yet) stay 0 until their ticket.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct PlayFlags(u16);

impl PlayFlags {
    /// No flag set.
    pub const EMPTY: Self = Self(0);
    /// Bit 0: the recording was favorited (`Favorited`) during the play.
    pub const FAVORITED: Self = Self(1 << 0);
    /// Bit 2: the listener sought backward (`SeekBack`) during the play.
    pub const SEEK_BACK: Self = Self(1 << 2);

    /// The stored bit set.
    #[must_use]
    pub const fn bits(self) -> u16 {
        self.0
    }

    /// Whether every flag of `other` is set.
    #[must_use]
    pub const fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }

    /// This set plus the flags of `other`.
    #[must_use]
    pub const fn with(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }
}

/// How an outcome event ended its play.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Ending {
    /// `Completed`, with the listened time it carries (deciseconds).
    Completed { listened_ds: u32 },
    /// `Skipped`, with the fraction it carries (per-255).
    Skipped { fraction_raw: u8 },
}

/// A newly stored `Completed` or `Skipped` event.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Outcome {
    /// `dict_device` id.
    pub device: i64,
    pub seq: i64,
    /// UTC ms.
    pub ts: i64,
    /// `dict_recording` id; `None` when the event names no recording.
    pub recording: Option<i64>,
    pub ending: Ending,
}

/// What [`write_play`] did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PlayWrite {
    /// One `play` row was inserted.
    Written,
    /// No `PlayStarted` of the same recording precedes the outcome: no row.
    Unpaired,
}

/// What [`write_derived`] did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DerivedWrite {
    /// The value is in the `play` row it describes.
    Stored,
    /// No `play` row has that identity: nothing was written.
    NoPlay,
}

/// Sets `play.rate` of the play identified by device, session, recording and start time.
const UPDATE_PLAY_RATE: &str = "UPDATE play SET rate = ?1     WHERE recording = (SELECT id FROM dict_recording WHERE uuid = ?2) AND started_ts = ?3     AND device = (SELECT id FROM dict_device WHERE uuid = ?4)     AND session = (SELECT id FROM session WHERE uuid = ?5)";

/// The nearest earlier play-lifecycle event of the device: the start the outcome would pair with.
const FIND_BOUNDARY: &str = "SELECT e.seq, e.kind, e.session, e.recording, e.source, e.ctx, \
    e.ts_off + s.started_ts \
    FROM signal_event e JOIN session s ON s.id = e.session \
    WHERE e.device = ?1 AND e.seq < ?2 AND e.kind IN (?3, ?4, ?5) \
    ORDER BY e.seq DESC LIMIT 1";

/// What happened between the start and the outcome that the row records.
const FIND_WINDOW: &str = "SELECT e.kind, e.recording, e.ts_off + s.started_ts \
    FROM signal_event e JOIN session s ON s.id = e.session \
    WHERE e.device = ?1 AND e.seq > ?2 AND e.seq < ?3 AND e.kind IN (?4, ?5, ?6, ?7) \
    ORDER BY e.seq";

/// Whether the play of a start was already written (see [`has_play_row`]).
const FIND_PLAY_OF_START: &str = "SELECT EXISTS (SELECT 1 FROM play \
    WHERE recording = ?1 AND started_ts = ?2 AND device = ?3 AND session = ?4)";

const FIND_CONTEXT_KEY: &str = "SELECT key FROM context_snapshot WHERE id = ?1";

const INSERT_PLAY: &str = "INSERT INTO play \
    (device, session, recording, source, started_ts, listened_ds, end_reason, end_frac, flags, \
     peak_section, context_key) \
    VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, NULL, ?10)";

/// The start row an outcome pairs with.
struct Start {
    seq: i64,
    kind: u8,
    session: i64,
    recording: Option<i64>,
    source: Option<i64>,
    ctx: Option<i64>,
    ts: i64,
}

/// Writes the `play` row of `outcome` in `tx`, if its start is found (module docs).
pub(crate) fn write_play(tx: &Transaction<'_>, outcome: &Outcome) -> Result<PlayWrite, DbError> {
    let Some(recording) = outcome.recording else {
        return Ok(PlayWrite::Unpaired);
    };
    let start = tx
        .prepare_cached(FIND_BOUNDARY)
        .map_err(DbError::sqlite("preparing the play start lookup"))?
        .query_row(
            params![
                outcome.device,
                outcome.seq,
                SignalKind::PlayStarted.discriminant(),
                SignalKind::Skipped.discriminant(),
                SignalKind::Completed.discriminant(),
            ],
            |row| {
                Ok(Start {
                    seq: row.get(0)?,
                    kind: row.get(1)?,
                    session: row.get(2)?,
                    recording: row.get(3)?,
                    source: row.get(4)?,
                    ctx: row.get(5)?,
                    ts: row.get(6)?,
                })
            },
        )
        .optional()
        .map_err(DbError::sqlite("looking up the start of a play"))?;
    let Some(start) = start.filter(|s| {
        s.kind == SignalKind::PlayStarted.discriminant() && s.recording == Some(recording)
    }) else {
        return Ok(PlayWrite::Unpaired);
    };
    if has_play_row(tx, outcome.device, &start, recording)? {
        // The start's play already has its row: this outcome is not its outcome (module docs).
        return Ok(PlayWrite::Unpaired);
    }

    let (paused_ms, flags) = scan_window(tx, outcome, &start, recording)?;
    let (listened_ds, end_reason, end_frac) = match outcome.ending {
        Ending::Completed { listened_ds } => (listened_ds, EndReason::Completed, u8::MAX),
        Ending::Skipped { fraction_raw } => {
            let listened_ms = (outcome.ts - start.ts - paused_ms).max(0);
            let listened_ds = PositionDs::from_ms(listened_ms.unsigned_abs()).ds();
            (
                listened_ds,
                EndReason::of_skip(listened_ds, fraction_raw),
                fraction_raw,
            )
        }
    };
    let context_key = context_key(tx, &start)?;

    tx.prepare_cached(INSERT_PLAY)
        .map_err(DbError::sqlite("preparing the play insert"))?
        .execute(params![
            outcome.device,
            start.session,
            recording,
            start.source,
            start.ts,
            listened_ds,
            end_reason.code(),
            end_frac,
            flags.bits(),
            context_key,
        ])
        .map_err(DbError::sqlite("inserting a play record"))?;
    Ok(PlayWrite::Written)
}

/// Stores `value` into the `play` row of the play it describes, if there is one (module docs).
///
/// The `match` has no wildcard: a new derived payload does not compile until it is decided here
/// which column it fills.
pub(crate) fn write_derived(
    tx: &Transaction<'_>,
    value: &DerivedValue,
) -> Result<DerivedWrite, DbError> {
    let (sql, stored) = match value.payload {
        DerivedPayload::RateSession(p) => (UPDATE_PLAY_RATE, p.rate.raw()),
    };
    let play = value.play;
    let changed = tx
        .prepare_cached(sql)
        .map_err(DbError::sqlite("preparing the update of a play record"))?
        .execute(params![
            stored,
            &play.recording.as_bytes()[..],
            play.started.as_i64(),
            &value.id.device.as_bytes()[..],
            &play.session.as_bytes()[..],
        ])
        .map_err(DbError::sqlite("storing a derived value in a play record"))?;
    Ok(if changed == 0 {
        DerivedWrite::NoPlay
    } else {
        DerivedWrite::Stored
    })
}

/// Whether `start` already has its `play` row. The row's identity is what it copies from the
/// start: device, session, recording and start time; `play_rec_ts` makes the lookup an index seek.
fn has_play_row(
    tx: &Transaction<'_>,
    device: i64,
    start: &Start,
    recording: i64,
) -> Result<bool, DbError> {
    tx.prepare_cached(FIND_PLAY_OF_START)
        .map_err(DbError::sqlite("preparing the lookup of a play record"))?
        .query_row(params![recording, start.ts, device, start.session], |row| {
            row.get(0)
        })
        .map_err(DbError::sqlite("looking up the play record of a start"))
}

/// Reads the events between the start and the outcome: how long the play was paused, and which
/// flags its events set.
fn scan_window(
    tx: &Transaction<'_>,
    outcome: &Outcome,
    start: &Start,
    recording: i64,
) -> Result<(i64, PlayFlags), DbError> {
    const PAUSED: u8 = SignalKind::Paused.discriminant();
    const RESUMED: u8 = SignalKind::Resumed.discriminant();
    const FAVORITED: u8 = SignalKind::Favorited.discriminant();
    const SEEK_BACK: u8 = SignalKind::SeekBack.discriminant();
    const READ: &str = "reading the events of a play";
    let mut stmt = tx
        .prepare_cached(FIND_WINDOW)
        .map_err(DbError::sqlite("preparing the play window lookup"))?;
    let mut rows = stmt
        .query(params![
            outcome.device,
            start.seq,
            outcome.seq,
            SignalKind::Paused.discriminant(),
            SignalKind::Resumed.discriminant(),
            SignalKind::Favorited.discriminant(),
            SignalKind::SeekBack.discriminant(),
        ])
        .map_err(DbError::sqlite("reading the events of a play"))?;
    let mut paused_ms = 0i64;
    let mut paused_since: Option<i64> = None;
    let mut flags = PlayFlags::EMPTY;
    while let Some(row) = rows
        .next()
        .map_err(DbError::sqlite("reading the events of a play"))?
    {
        let kind: u8 = row.get(0).map_err(DbError::sqlite(READ))?;
        let event_recording: Option<i64> = row.get(1).map_err(DbError::sqlite(READ))?;
        let ts: i64 = row.get(2).map_err(DbError::sqlite(READ))?;
        // Everything between the start and the outcome of one device is about this play, so
        // pauses need no recording; a favorite or a seek names the recording it is about.
        if kind == PAUSED {
            paused_since.get_or_insert(ts);
        } else if kind == RESUMED {
            if let Some(since) = paused_since.take() {
                paused_ms += (ts - since).max(0);
            }
        } else if event_recording == Some(recording) {
            if kind == FAVORITED {
                flags = flags.with(PlayFlags::FAVORITED);
            } else if kind == SEEK_BACK {
                flags = flags.with(PlayFlags::SEEK_BACK);
            }
        }
    }
    // Paused when the play was left: that time was not listening either.
    if let Some(since) = paused_since {
        paused_ms += (outcome.ts - since).max(0);
    }
    Ok((paused_ms, flags))
}

/// The packed context key of the play, or 0 (nothing known).
///
/// 0 when the start carries no snapshot id. A snapshot id with no row, or a stored key that is
/// not a valid [`ContextKey`], is a producer fault: it is logged (never the values) and the play
/// stores 0 instead of a key nobody can read.
fn context_key(tx: &Transaction<'_>, start: &Start) -> Result<u32, DbError> {
    let Some(ctx) = start.ctx else {
        return Ok(0);
    };
    let stored: Option<i64> = tx
        .prepare_cached(FIND_CONTEXT_KEY)
        .map_err(DbError::sqlite("preparing the context key lookup"))?
        .query_row([ctx], |row| row.get(0))
        .optional()
        .map_err(DbError::sqlite("reading the context key of a play"))?;
    let Some(stored) = stored else {
        tracing::warn!("play start names a context snapshot that does not exist; context key 0");
        return Ok(0);
    };
    match u32::try_from(stored)
        .ok()
        .and_then(|raw| ContextKey::from_raw(raw).ok().map(ContextKey::raw))
    {
        Some(raw) => Ok(raw),
        None => {
            tracing::warn!("context snapshot holds an unusable context key; context key 0");
            Ok(0)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn end_reason_codes_are_frozen_and_round_trip() {
        let codes: Vec<u8> = EndReason::ALL.iter().map(|r| r.code()).collect();
        assert_eq!(codes, [0, 1, 2, 3, 4, 5, 6]);
        for r in EndReason::ALL {
            assert_eq!(EndReason::from_code(r.code()), Some(r));
        }
        assert_eq!(EndReason::from_code(7), None);
    }

    #[test]
    fn a_skip_is_early_mid_or_late() {
        let skip = EndReason::of_skip;
        // Under 30 s listened, in the first 80 %: early.
        assert_eq!(skip(0, 0), EndReason::SkippedEarly);
        assert_eq!(skip(299, 100), EndReason::SkippedEarly);
        // 30 s or more, up to 80 %: mid.
        assert_eq!(skip(300, 100), EndReason::SkippedMid);
        assert_eq!(skip(1_800, 204), EndReason::SkippedMid);
        // Past 80 % is late, whatever was listened (a short track).
        assert_eq!(skip(1_800, 205), EndReason::SkippedLate);
        assert_eq!(skip(50, 250), EndReason::SkippedLate);
    }

    #[test]
    fn flag_bits_follow_the_documented_order() {
        assert_eq!(PlayFlags::FAVORITED.bits(), 0b001);
        assert_eq!(PlayFlags::SEEK_BACK.bits(), 0b100);
        let both = PlayFlags::EMPTY
            .with(PlayFlags::FAVORITED)
            .with(PlayFlags::SEEK_BACK);
        assert!(both.contains(PlayFlags::FAVORITED) && both.contains(PlayFlags::SEEK_BACK));
        assert_eq!(both.bits(), 0b101);
    }
}
