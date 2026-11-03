//! Wiping signal data: one signal category, or everything in `signals.db`.
//!
//! The statements of the wipes of [`SignalWiper`](crate::SignalWiper); this module owns the rule
//! for **what is derived from what** (`docs/rules/privacy-and-signals.md`, "Wipe"), written as
//! tables so the rule is read, reviewed and tested in one place.
//!
//! # A category wipe
//!
//! A [`SignalCategory`] is a catalog section (`SignalCategory::of` maps every kind to one). Wiping
//! it deletes, in one transaction:
//!
//! - its `signal_event` rows and its `ui_counter` rows (the counters of the Browsing category);
//! - **what is derived from them**. The play table and the monthly rollups are folded from the
//!   signals of several categories. The rule is the one of the context wipe: **a column is cleared
//!   by the wipe of every category it is derived from, and nothing else changes**. A derived
//!   column that blends several categories (a score) is cleared by each of them, like the activity
//!   guess of the context key. A row is deleted when what identifies it was wiped, or when its
//!   cleared columns were all that it carried.
//!
//! | table | derived from | cleared by the wipe of | cleared to |
//! |---|---|---|---|
//! | `play` row (its existence, `recording`, `source`, `started_ts`, `session`) | `PlayStarted` | queue and library | the row is **deleted** |
//! | `play.listened_ds`, `end_reason`, `end_frac` | `Completed`, `Skipped`, `Paused`, `Resumed` | seek, skip, replay | 0, [`EndReason::Unknown`], 0 |
//! | `play.flags` | one flag per source signal, see [`FLAG_SOURCES`] | the flag's category | the bit is 0 |
//! | `play.peak_section` | section-carrying signals | volume, speed, seek/skip/replay | NULL |
//! | `rollup_monthly` row | `PlayStarted` | queue and library | the row is **deleted** |
//! | `rollup_monthly.plays` | `PlayStarted` | queue and library | 0 |
//! | `completes`, `skips_early`, `skips_late`, `listened_s` | `Completed`, `Skipped` | seek, skip, replay | 0 |
//! | `fav_delta` | `Favorited`, `Unfavorited` | queue and library | 0 |
//! | `score_sum` | every category (the weights of the catalog) | **every** category | 0 |
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

/// The category whose `PlayStarted` identifies a `play` row and a `rollup_monthly` row: wiping it
/// deletes them (module docs).
const ROW_IDENTITY: SignalCategory = SignalCategory::QueueLibrary;

/// A column that is derived from signals and the value it takes when they are wiped.
struct Derived {
    column: &'static str,
    /// SQL literal the column is set to.
    cleared: &'static str,
    /// The wipe of any of these categories clears it.
    from: &'static [SignalCategory],
}

const SEEK: SignalCategory = SignalCategory::SeekSkipReplay;

/// The derived columns of `play` (not `flags`, see [`FLAG_SOURCES`]; not `context_key`, which is
/// context). `end_reason` is [`EndReason::Unknown`]'s code, pinned by a test.
const PLAY_COLUMNS: &[Derived] = &[
    Derived {
        column: "listened_ds",
        cleared: "0",
        from: &[SEEK],
    },
    Derived {
        column: "end_reason",
        cleared: "6",
        from: &[SEEK],
    },
    Derived {
        column: "end_frac",
        cleared: "0",
        from: &[SEEK],
    },
    // Filled by the audio analysis from the sections of volume, rate and seek signals.
    Derived {
        column: "peak_section",
        cleared: "NULL",
        from: &[
            SignalCategory::Volume,
            SignalCategory::Speed,
            SignalCategory::SeekSkipReplay,
        ],
    },
];

/// The category whose signals set each `play.flags` bit: the bit order of
/// `docs/rules/storage-budget.md`. A wipe of the category clears the bit. The three flags of the
/// queue and library category go with their row ([`ROW_IDENTITY`]); they are listed so that no
/// documented bit is without an owner, which a test checks.
const FLAG_SOURCES: [(u8, SignalCategory); 11] = [
    (0, SignalCategory::QueueLibrary),   // favorited: Favorited
    (1, SignalCategory::SeekSkipReplay), // replayed: Replayed
    (2, SignalCategory::SeekSkipReplay), // seek_back: SeekBack
    (3, SignalCategory::SeekSkipReplay), // seek_fwd_intro: SeekForward
    (4, SignalCategory::Volume),         // vol_up_early: VolumeChanged
    (5, SignalCategory::Volume),         // vol_down: VolumeChanged
    (6, SignalCategory::Speed),          // rate_up: RateChanged
    (7, SignalCategory::Speed),          // rate_down: RateChanged
    (8, SignalCategory::Browsing),       // lyrics: ViewOpened
    (9, SignalCategory::QueueLibrary),   // queued: QueuedNext, QueuedLater
    (10, SignalCategory::QueueLibrary),  // removed_after: RemovedFromQueue
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
        from: &[SEEK],
    },
    Derived {
        column: "skips_early",
        cleared: "0",
        from: &[SEEK],
    },
    Derived {
        column: "skips_late",
        cleared: "0",
        from: &[SEEK],
    },
    Derived {
        column: "fav_delta",
        cleared: "0",
        from: &[ROW_IDENTITY],
    },
    Derived {
        column: "listened_s",
        cleared: "0",
        from: &[SEEK],
    },
    // The sum of the weights of every signal: it cannot be split by category.
    Derived {
        column: "score_sum",
        cleared: "0.0",
        from: &SignalCategory::ALL,
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
    let mut report = SignalWipeReport::default();
    if categories.is_empty() {
        return Ok(report);
    }
    let kinds = kind_codes(categories);
    report.events_deleted = tx
        .execute(
            &format!("DELETE FROM signal_event WHERE kind IN ({kinds})"),
            [],
        )
        .map_err(DbError::sqlite("deleting the events of a category"))?;
    report.counter_rows_deleted = tx
        .execute(
            &format!("DELETE FROM ui_counter WHERE kind IN ({kinds})"),
            [],
        )
        .map_err(DbError::sqlite("deleting the counters of a category"))?;

    if categories.contains(&ROW_IDENTITY) {
        report.plays_deleted = delete_all(tx, "play")?;
        report.rollups_deleted = delete_all(tx, "rollup_monthly")?;
    } else {
        report.plays_cleared = clear_plays(tx, categories)?;
        let (deleted, cleared) = clear_rollups(tx, categories)?;
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

/// The `kind` codes of every kind of `categories`, as a comma-separated list for `IN (...)`.
fn kind_codes(categories: &[SignalCategory]) -> String {
    SignalKind::ALL
        .iter()
        .filter(|kind| categories.contains(&SignalCategory::of(**kind)))
        .map(|kind| kind.discriminant().to_string())
        .collect::<Vec<_>>()
        .join(",")
}

/// The columns of `table` that a wipe of `categories` clears.
fn cleared<'a>(table: &'a [Derived], categories: &[SignalCategory]) -> Vec<&'a Derived> {
    table
        .iter()
        .filter(|d| d.from.iter().any(|c| categories.contains(c)))
        .collect()
}

/// `play.flags` bits that a wipe of `categories` clears.
fn flag_mask(categories: &[SignalCategory]) -> u16 {
    FLAG_SOURCES
        .iter()
        .filter(|(_, owner)| categories.contains(owner))
        .fold(0, |mask, (bit, _)| mask | (1u16 << bit))
}

/// Clears the derived columns of the `play` rows that have any, in one `UPDATE` that writes only
/// the rows that change. Returns how many.
fn clear_plays(tx: &Transaction<'_>, categories: &[SignalCategory]) -> Result<usize, DbError> {
    let columns = cleared(PLAY_COLUMNS, categories);
    let mask = flag_mask(categories);
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
fn clear_rollups(
    tx: &Transaction<'_>,
    categories: &[SignalCategory],
) -> Result<(usize, usize), DbError> {
    let columns = cleared(ROLLUP_COLUMNS, categories);
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

    /// Every flag bit of `storage-budget.md` (0 to 10) has exactly one owning category, and the
    /// two flags the play writer sets today are owned by the category of the signal they come from.
    #[test]
    fn every_documented_play_flag_has_one_owner() {
        let bits: Vec<u8> = FLAG_SOURCES.iter().map(|(bit, _)| *bit).collect();
        assert_eq!(bits, (0..=10).collect::<Vec<u8>>());
        let owner = |flag: crate::PlayFlags| {
            FLAG_SOURCES
                .iter()
                .find(|(bit, _)| 1u16 << bit == flag.bits())
                .map(|(_, c)| *c)
        };
        assert_eq!(
            owner(crate::PlayFlags::FAVORITED),
            Some(SignalCategory::of(SignalKind::Favorited))
        );
        assert_eq!(
            owner(crate::PlayFlags::SEEK_BACK),
            Some(SignalCategory::of(SignalKind::SeekBack))
        );
    }

    /// The outcome signals are all seek/skip/replay, the start is the row identity: the two facts
    /// the module docs build the play-row rule on.
    #[test]
    fn the_outcome_signals_belong_to_the_category_that_clears_the_outcome_columns() {
        for kind in [
            SignalKind::Completed,
            SignalKind::Skipped,
            SignalKind::Paused,
            SignalKind::Resumed,
        ] {
            assert_eq!(SignalCategory::of(kind), SEEK, "{kind}");
        }
        assert_eq!(
            SignalCategory::of(SignalKind::PlayStarted),
            ROW_IDENTITY,
            "the start identifies the row"
        );
    }

    #[test]
    fn every_category_has_kinds_and_the_codes_are_a_plain_list() {
        for category in SignalCategory::ALL {
            let codes = kind_codes(&[category]);
            assert!(!codes.is_empty(), "{category:?}");
            assert!(codes.bytes().all(|b| b.is_ascii_digit() || b == b','));
        }
    }
}
