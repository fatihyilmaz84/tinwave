//! Wiping signal data: signal categories, privacy tiers, or everything in `signals.db`.
//!
//! The statements of the wipes of [`SignalWiper`](crate::SignalWiper); this module owns the rule
//! for **what is derived from what** (`docs/rules/privacy-and-signals.md`, "Wipe"), written as
//! tables so the rule is read, reviewed and tested in one place.
//!
//! # A wipe of signal kinds
//!
//! Every wipe of signals is a wipe of a **set of [`SignalKind`]s**: a category wipe is the kinds
//! of a [`SignalCategory`] (a catalog section, `SignalCategory::of` maps every kind to one), a
//! tier wipe is the kinds whose `SignalKind::tier` is in the wiped tiers. It deletes, in one
//! transaction:
//!
//! - those kinds' `signal_event` rows and `ui_counter` rows (the counters of the Browsing category);
//! - **what is derived from them**. The play table and the monthly rollups are folded from the
//!   signals of several kinds. The rule is the one of the context wipe: **a column is cleared by
//!   the wipe of any signal it is derived from, and nothing else changes**. A derived column that
//!   blends many signals (a score) is cleared by each of them, like the activity guess of the
//!   context key. A row is deleted when what identifies it was wiped, or when its cleared columns
//!   were all that it carried.
//!
//! The sources are kinds (not categories) so that a tier, which cuts across categories, clears
//! exactly what its own signals feed. A category wipe gives the same result as it did when the
//! sources were categories, because it wipes all of the category's kinds.
//!
//! | table | derived from (kinds) | cleared to |
//! |---|---|---|
//! | `play` row (its existence, `recording`, `source`, `started_ts`, `session`) | `PlayStarted` | the row is **deleted** |
//! | `play.listened_ds`, `end_reason`, `end_frac` | [`OUTCOME`]: `Completed`, `Skipped`, `Paused`, `Resumed` | 0, [`EndReason::Unknown`], 0 |
//! | `play.flags` | one flag per source signal, see [`FLAG_SOURCES`] | the bit is 0 |
//! | `play.peak_section` | [`SECTIONED`]: the signals that carry a section | NULL |
//! | `rollup_monthly` row | `PlayStarted` | the row is **deleted** |
//! | `rollup_monthly.plays` | `PlayStarted` | 0 |
//! | `completes`, `skips_early`, `skips_late`, `listened_s` | [`OUTCOME`] | 0 |
//! | `fav_delta` | `Favorited`, `Unfavorited` | 0 |
//! | `score_sum` | every kind (the weights of the catalog) | 0 |
//!
//! For a tier wipe the sets are small today: T0 is every kind except `WindowState`,
//! `OutputDeviceChanged` and the derived `PerceivedLevel` (T1); T2 and T3 have no signal kind, so
//! the signal part of their wipe changes nothing (the context fields of the tier are wiped in the
//! same transaction, `SignalWiper::wipe_tiers`).
//!
//! `play.context_key` and `context_snapshot.key` are context, wiped by the context categories
//! (`SignalWiper::wipe_context`), not by a signal category. The wipe of a category does not touch
//! `context_snapshot`, `session` or the dictionaries: they are shared by the events of every
//! category and hold no signal of their own; only [wipe everything](wipe_everything) clears them.
//!
//! Why a play row is **deleted** by the queue and library wipe and only **cleared** by the seek,
//! skip and replay wipe: the row exists because a `PlayStarted` (catalog section 4) named the
//! recording and the time; the outcome columns only describe how that play ended. Clearing the
//! outcome leaves what the listener did not ask to wipe (what was played, when); clearing the
//! start leaves an outcome with no recording and no time, which is nothing. A rollup row left with
//! only zero counters is deleted for the same reason.
//!
//! The monthly play count is the same fact as the play row: *that* a recording was played, which
//! the start names. So the seek, skip and replay wipe keeps `rollup_monthly.plays` (the cold tier
//! keeps "what was played and when", like the warm tier) and only the queue and library wipe
//! clears it. A rollup row is then deleted by the seek wipe only when `plays` and `fav_delta` are
//! both 0 (nothing but the cleared outcome counters was in it).
//!
//! A kind that no build knows (a code from a newer build) belongs to no category and is removed
//! only by [wipe everything](wipe_everything).
//!
//! # Wipe everything
//!
//! Deletes every row of `signal_event`, `ui_counter`, `play`, `rollup_monthly`,
//! `context_snapshot`, `session` and the four dictionaries: the whole signal history, down to the
//! times and the ids of the recordings. It keeps the schema, the migration record and
//! `signal_settings`: the Signal Center toggles are a preference, not collected data, and a wipe
//! must not turn collection back on or off.
//!
//! `taste.db` does not exist yet, so nothing of it is covered here. The ticket that creates it
//! must extend the wipe in the same transaction scheme (a second database cannot share one
//! transaction: it needs its own all-or-nothing rule, decided there).
//!
//! # The writer keeps running
//!
//! The writer's connection is not told about a wipe. Wipe everything bumps the **wipe epoch**
//! (`PRAGMA user_version` of `signals.db`, unused by anything else; [`wipe_epoch`]) in its
//! transaction, and [`SignalStore`](crate::SignalStore) reads it at the start of every batch, under
//! the write lock, and forgets its dictionary and session caches when it changed: without that,
//! after a wipe of the dictionaries its cached integers would point at rows that no longer exist,
//! or (SQLite reuses the integers of an emptied table) at another recording. Only a wipe of the
//! rows the caches point at (the dictionaries and `session`) changes the epoch: a category wipe, a
//! context wipe and a settings toggle leave them in place, so they keep the caches. (SQLite's
//! `data_version` was tried first and rejected: it changes on **every** commit of another
//! connection, which cleared the caches before almost every batch in the 90-day growth run, +30 %
//! writer CPU.) Anything new that deletes dictionary or session rows must bump the epoch too.
//!
//! A play in progress when its `PlayStarted` was wiped ends without a `play` row (the outcome has
//! no start to pair with: `PlayWrite::Unpaired`, logged).
//!
//! # Cost and locking
//!
//! As the context wipe: one transaction, and every table is touched by a fixed number of
//! statements, each one scan, whatever the number of rows or kinds. `signal_event` and
//! `ui_counter` have no index on `kind`, so the delete is one scan of each (the hot tier is 90
//! days); `play` has no index on its columns either and gets none for this. The connection runs
//! with `secure_delete`, so deleted content is zeroed in the pages, not left in the free list; the
//! zeroed pages reach the main database file when the WAL is checkpointed, which the wiper tries
//! right after the commit (best effort, `SignalWiper`).

use rusqlite::{Connection, Transaction};
use tinwave_core::Tier;
use tinwave_signals::{SignalCategory, SignalKind};

use crate::DbError;

/// What a signal wipe changed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct SignalWipeReport {
    /// `signal_event` rows deleted.
    pub events_deleted: usize,
    /// `ui_counter` rows deleted.
    pub counter_rows_deleted: usize,
    /// `play` rows deleted.
    pub plays_deleted: usize,
    /// `play` rows that kept their identity and lost a column or a flag.
    pub plays_cleared: usize,
    /// `rollup_monthly` rows deleted.
    pub rollups_deleted: usize,
    /// `rollup_monthly` rows that lost a column and kept the rest.
    pub rollups_cleared: usize,
    /// `context_snapshot` rows deleted (wipe everything only).
    pub snapshots_deleted: usize,
    /// `session` rows deleted (wipe everything only).
    pub sessions_deleted: usize,
    /// Rows deleted from the four dictionaries (wipe everything only).
    pub dictionary_rows_deleted: usize,
}

/// The signal whose `PlayStarted` identifies a `play` row and a `rollup_monthly` row: wiping it
/// deletes them (module docs).
const ROW_IDENTITY: SignalKind = SignalKind::PlayStarted;

/// How a play ended: the signals the outcome columns are folded from.
const OUTCOME: &[SignalKind] = &[
    SignalKind::Completed,
    SignalKind::Skipped,
    SignalKind::Paused,
    SignalKind::Resumed,
];

/// The signals whose payload carries a section of the recording; the audio analysis fills
/// `play.peak_section` from them.
const SECTIONED: &[SignalKind] = &[
    SignalKind::VolumeChanged,
    SignalKind::SystemVolumeChanged,
    SignalKind::RateChanged,
    SignalKind::Skipped,
    SignalKind::SeekForward,
    SignalKind::SeekBack,
];

/// A column that is derived from signals and the value it takes when they are wiped.
struct Derived {
    column: &'static str,
    /// SQL literal the column is set to.
    cleared: &'static str,
    /// The wipe of any of these kinds clears it.
    from: &'static [SignalKind],
}

/// The derived columns of `play` (not `flags`, see [`FLAG_SOURCES`]; not `context_key`, which is
/// context). `end_reason` is [`EndReason::Unknown`]'s code, pinned by a test.
const PLAY_COLUMNS: &[Derived] = &[
    Derived {
        column: "listened_ds",
        cleared: "0",
        from: OUTCOME,
    },
    Derived {
        column: "end_reason",
        cleared: "6",
        from: OUTCOME,
    },
    Derived {
        column: "end_frac",
        cleared: "0",
        from: OUTCOME,
    },
    Derived {
        column: "peak_section",
        cleared: "NULL",
        from: SECTIONED,
    },
];

/// The signals that set each `play.flags` bit: the bit order of `docs/rules/storage-budget.md`. A
/// wipe of any of them clears the bit. The three flags of the queue and library category go with
/// their row when the row's start is wiped ([`ROW_IDENTITY`]); they are listed so that no
/// documented bit is without a source, which a test checks.
const FLAG_SOURCES: [(u8, &[SignalKind]); 11] = [
    (0, &[SignalKind::Favorited]),                           // favorited
    (1, &[SignalKind::Replayed]),                            // replayed
    (2, &[SignalKind::SeekBack]),                            // seek_back
    (3, &[SignalKind::SeekForward]),                         // seek_fwd_intro
    (4, &[SignalKind::VolumeChanged]),                       // vol_up_early
    (5, &[SignalKind::VolumeChanged]),                       // vol_down
    (6, &[SignalKind::RateChanged]),                         // rate_up
    (7, &[SignalKind::RateChanged]),                         // rate_down
    (8, &[SignalKind::ViewOpened]),                          // lyrics
    (9, &[SignalKind::QueuedNext, SignalKind::QueuedLater]), // queued
    (10, &[SignalKind::RemovedFromQueue]),                   // removed_after
];

/// The derived columns of `rollup_monthly`.
const ROLLUP_COLUMNS: &[Derived] = &[
    // The number of `PlayStarted`: an outcome only says how the play ended (module docs).
    Derived {
        column: "plays",
        cleared: "0",
        from: &[ROW_IDENTITY],
    },
    Derived {
        column: "completes",
        cleared: "0",
        from: OUTCOME,
    },
    Derived {
        column: "skips_early",
        cleared: "0",
        from: OUTCOME,
    },
    Derived {
        column: "skips_late",
        cleared: "0",
        from: OUTCOME,
    },
    Derived {
        column: "fav_delta",
        cleared: "0",
        from: &[SignalKind::Favorited, SignalKind::Unfavorited],
    },
    Derived {
        column: "listened_s",
        cleared: "0",
        from: OUTCOME,
    },
    // The sum of the weights of every signal: it cannot be split by kind.
    Derived {
        column: "score_sum",
        cleared: "0.0",
        from: SignalKind::ALL,
    },
];

/// Tables emptied by [`wipe_everything`], children first (`session` references `dict_device`).
const EVERYTHING: [&str; 10] = [
    "signal_event",
    "ui_counter",
    "play",
    "rollup_monthly",
    "context_snapshot",
    "session",
    "dict_entity",
    "dict_source",
    "dict_recording",
    "dict_device",
];

/// Wipes the signals of `categories` and what is derived from them, in `tx`.
pub(crate) fn wipe_categories(
    tx: &Transaction<'_>,
    categories: &[SignalCategory],
) -> Result<SignalWipeReport, DbError> {
    let kinds: Vec<SignalKind> = SignalKind::ALL
        .iter()
        .copied()
        .filter(|kind| categories.contains(&SignalCategory::of(*kind)))
        .collect();
    wipe_kinds(tx, &kinds)
}

/// Wipes the signals of every kind whose tier is in `tiers` and what is derived from them, in
/// `tx`. A tier without a signal kind (T2, T3 today) changes nothing.
pub(crate) fn wipe_tiers(
    tx: &Transaction<'_>,
    tiers: &[Tier],
) -> Result<SignalWipeReport, DbError> {
    let kinds: Vec<SignalKind> = SignalKind::ALL
        .iter()
        .copied()
        .filter(|kind| tiers.contains(&kind.tier()))
        .collect();
    wipe_kinds(tx, &kinds)
}

/// Wipes the signals of `kinds` and what is derived from them (module docs), in `tx`.
fn wipe_kinds(tx: &Transaction<'_>, kinds: &[SignalKind]) -> Result<SignalWipeReport, DbError> {
    let mut report = SignalWipeReport::default();
    if kinds.is_empty() {
        return Ok(report);
    }
    let codes = kind_codes(kinds);
    report.events_deleted = tx
        .execute(
            &format!("DELETE FROM signal_event WHERE kind IN ({codes})"),
            [],
        )
        .map_err(DbError::sqlite("deleting the events of the wiped kinds"))?;
    report.counter_rows_deleted = tx
        .execute(
            &format!("DELETE FROM ui_counter WHERE kind IN ({codes})"),
            [],
        )
        .map_err(DbError::sqlite("deleting the counters of the wiped kinds"))?;

    if kinds.contains(&ROW_IDENTITY) {
        report.plays_deleted = delete_all(tx, "play")?;
        report.rollups_deleted = delete_all(tx, "rollup_monthly")?;
    } else {
        report.plays_cleared = clear_plays(tx, kinds)?;
        let (deleted, cleared) = clear_rollups(tx, kinds)?;
        report.rollups_deleted = deleted;
        report.rollups_cleared = cleared;
    }
    Ok(report)
}

/// The wipe epoch: `PRAGMA user_version`, which only [`wipe_everything`] changes. The writer
/// compares it between batches to know that its cached dictionary and session ids are void.
///
/// Prepared once per connection (`prepare_cached`): the writer reads it before every batch.
pub(crate) fn wipe_epoch(conn: &Connection) -> Result<i32, DbError> {
    conn.prepare_cached("PRAGMA user_version")
        .map_err(DbError::sqlite("preparing the wipe epoch read"))?
        .query_row([], |row| row.get(0))
        .map_err(DbError::sqlite("reading the wipe epoch"))
}

/// Moves the epoch on, inside the wipe's transaction (it rolls back with the wipe). The value only
/// has to differ from the one the writer saw; it wraps.
fn bump_wipe_epoch(tx: &Transaction<'_>) -> Result<(), DbError> {
    let next = wipe_epoch(tx)?.wrapping_add(1);
    tx.pragma_update(None, "user_version", next)
        .map_err(DbError::sqlite("bumping the wipe epoch"))
}

/// Empties every signal table (module docs) and bumps the wipe epoch.
pub(crate) fn wipe_everything(tx: &Transaction<'_>) -> Result<SignalWipeReport, DbError> {
    bump_wipe_epoch(tx)?;
    let mut deleted = [0usize; EVERYTHING.len()];
    for (count, table) in deleted.iter_mut().zip(EVERYTHING) {
        *count = delete_all(tx, table)?;
    }
    Ok(SignalWipeReport {
        events_deleted: deleted[0],
        counter_rows_deleted: deleted[1],
        plays_deleted: deleted[2],
        rollups_deleted: deleted[3],
        snapshots_deleted: deleted[4],
        sessions_deleted: deleted[5],
        dictionary_rows_deleted: deleted[6..].iter().sum(),
        ..SignalWipeReport::default()
    })
}

fn delete_all(tx: &Transaction<'_>, table: &'static str) -> Result<usize, DbError> {
    tx.execute(&format!("DELETE FROM {table}"), [])
        .map_err(DbError::sqlite("emptying a signal table"))
}

/// The `kind` codes of `kinds`, as a comma-separated list for `IN (...)`.
fn kind_codes(kinds: &[SignalKind]) -> String {
    kinds
        .iter()
        .map(|kind| kind.discriminant().to_string())
        .collect::<Vec<_>>()
        .join(",")
}

/// The columns of `table` that a wipe of `kinds` clears.
fn cleared<'a>(table: &'a [Derived], kinds: &[SignalKind]) -> Vec<&'a Derived> {
    table
        .iter()
        .filter(|d| d.from.iter().any(|k| kinds.contains(k)))
        .collect()
}

/// `play.flags` bits that a wipe of `kinds` clears.
fn flag_mask(kinds: &[SignalKind]) -> u16 {
    FLAG_SOURCES
        .iter()
        .filter(|(_, sources)| sources.iter().any(|k| kinds.contains(k)))
        .fold(0, |mask, (bit, _)| mask | (1u16 << bit))
}

/// Clears the derived columns of the `play` rows that have any, in one `UPDATE` that writes only
/// the rows that change. Returns how many.
fn clear_plays(tx: &Transaction<'_>, kinds: &[SignalKind]) -> Result<usize, DbError> {
    let columns = cleared(PLAY_COLUMNS, kinds);
    let mask = flag_mask(kinds);
    if columns.is_empty() && mask == 0 {
        return Ok(0);
    }
    let mut set: Vec<String> = columns
        .iter()
        .map(|d| format!("{} = {}", d.column, d.cleared))
        .collect();
    let mut changes: Vec<String> = columns
        .iter()
        .map(|d| format!("{} IS NOT {}", d.column, d.cleared))
        .collect();
    if mask != 0 {
        set.push(format!("flags = flags & {}", !mask));
        changes.push(format!("flags & {mask} != 0"));
    }
    tx.execute(
        &format!(
            "UPDATE play SET {} WHERE {}",
            set.join(", "),
            changes.join(" OR ")
        ),
        [],
    )
    .map_err(DbError::sqlite(
        "clearing the derived columns of play records",
    ))
}

/// Clears the derived columns of `rollup_monthly`. First deletes the rows that carried nothing
/// else (every column that stays is 0), then clears the rest. Returns `(deleted, cleared)`.
fn clear_rollups(tx: &Transaction<'_>, kinds: &[SignalKind]) -> Result<(usize, usize), DbError> {
    let columns = cleared(ROLLUP_COLUMNS, kinds);
    if columns.is_empty() {
        return Ok((0, 0));
    }
    let changes = columns
        .iter()
        .map(|d| format!("{} IS NOT {}", d.column, d.cleared))
        .collect::<Vec<_>>()
        .join(" OR ");
    let kept_zero = ROLLUP_COLUMNS
        .iter()
        .filter(|d| !columns.iter().any(|c| c.column == d.column))
        .map(|d| format!("{} = 0", d.column))
        .collect::<Vec<_>>();
    let kept_zero = if kept_zero.is_empty() {
        "1".to_owned()
    } else {
        kept_zero.join(" AND ")
    };
    let deleted = tx
        .execute(
            &format!("DELETE FROM rollup_monthly WHERE ({changes}) AND ({kept_zero})"),
            [],
        )
        .map_err(DbError::sqlite("deleting emptied monthly rollups"))?;
    let set = columns
        .iter()
        .map(|d| format!("{} = {}", d.column, d.cleared))
        .collect::<Vec<_>>()
        .join(", ");
    let updated = tx
        .execute(
            &format!("UPDATE rollup_monthly SET {set} WHERE {changes}"),
            [],
        )
        .map_err(DbError::sqlite(
            "clearing the derived columns of monthly rollups",
        ))?;
    Ok((deleted, updated))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::play::EndReason;

    #[test]
    fn the_cleared_end_reason_is_the_unknown_code() {
        let column = PLAY_COLUMNS
            .iter()
            .find(|d| d.column == "end_reason")
            .unwrap();
        assert_eq!(column.cleared, EndReason::Unknown.code().to_string());
    }

    /// Every flag bit of `storage-budget.md` (0 to 10) has at least one source signal, and the two
    /// flags the play writer sets today come from the signal they are named after.
    #[test]
    fn every_documented_play_flag_has_a_source() {
        let bits: Vec<u8> = FLAG_SOURCES.iter().map(|(bit, _)| *bit).collect();
        assert_eq!(bits, (0..=10).collect::<Vec<u8>>());
        assert!(FLAG_SOURCES.iter().all(|(_, kinds)| !kinds.is_empty()));
        let sources = |flag: crate::PlayFlags| {
            FLAG_SOURCES
                .iter()
                .find(|(bit, _)| 1u16 << bit == flag.bits())
                .map(|(_, kinds)| *kinds)
        };
        assert_eq!(
            sources(crate::PlayFlags::FAVORITED),
            Some(&[SignalKind::Favorited][..])
        );
        assert_eq!(
            sources(crate::PlayFlags::SEEK_BACK),
            Some(&[SignalKind::SeekBack][..])
        );
    }

    /// Every source is a stored kind (an event: a derived or counter kind feeds nothing here) and
    /// the sources are in the categories the category wipe tests expect, so a category wipe clears
    /// what it always did: the outcome signals are all seek/skip/replay, the start is queue and
    /// library, and the sectioned signals reach volume, speed and seek/skip/replay.
    #[test]
    fn the_source_signals_sit_in_the_categories_the_rule_documents() {
        let all_sources = PLAY_COLUMNS
            .iter()
            .chain(ROLLUP_COLUMNS)
            .filter(|d| d.column != "score_sum")
            .flat_map(|d| d.from.iter())
            .chain(FLAG_SOURCES.iter().flat_map(|(_, kinds)| kinds.iter()));
        for kind in all_sources {
            assert!(kind.is_event(), "{kind}");
        }
        for kind in OUTCOME {
            assert_eq!(
                SignalCategory::of(*kind),
                SignalCategory::SeekSkipReplay,
                "{kind}"
            );
        }
        assert_eq!(
            SignalCategory::of(ROW_IDENTITY),
            SignalCategory::QueueLibrary,
            "the start identifies the row"
        );
        // The categories of the sectioned signals are exactly these three, not "at least": a sectioned
        // signal of another category would make that category's wipe clear `peak_section` too, without
        // a test noticing.
        let expected = [
            SignalCategory::Volume,
            SignalCategory::Speed,
            SignalCategory::SeekSkipReplay,
        ];
        for kind in SECTIONED {
            assert!(
                expected.contains(&SignalCategory::of(*kind)),
                "{kind} is sectioned and belongs to {:?}",
                SignalCategory::of(*kind)
            );
        }
        for category in expected {
            assert!(
                SECTIONED.iter().any(|k| SignalCategory::of(*k) == category),
                "{category:?} has no sectioned signal"
            );
        }
    }

    #[test]
    fn the_kind_codes_are_a_plain_list() {
        for category in SignalCategory::ALL {
            let kinds: Vec<_> = SignalKind::ALL
                .iter()
                .copied()
                .filter(|k| SignalCategory::of(*k) == category)
                .collect();
            let codes = kind_codes(&kinds);
            assert!(!codes.is_empty(), "{category:?}");
            assert!(codes.bytes().all(|b| b.is_ascii_digit() || b == b','));
        }
    }
}
