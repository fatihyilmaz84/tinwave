//! User wipe of context data: [`SignalWiper`].
//!
//! The Signal Center wipes per category, per tier, or everything (`docs/rules/privacy-and-signals.md`).
//! This module is the part that removes **context** from `signals.db`: the context a play
//! happened in survives in two places, and a wipe of a context category must clear both, in one
//! transaction:
//!
//! - `context_snapshot.key` (hot tier): the packed `ContextKey` of each snapshot;
//! - `play.context_key` (warm tier, 3 years): the key a play record keeps.
//!
//! `rollup_monthly` carries no context and is not touched. Turning a category or tier **off**
//! (`PersistentSettings`) only stops new values; existing rows change only through a wipe here.
//!
//! # Which fields a category clears
//!
//! A [`ContextCategory`] is a provider group of the context engine
//! (`docs/guides/context-engine.md`) with the tier of `signal-catalog.md` section 7. Every
//! `ContextKey` field has **exactly one owning category**, the provider the value is read from.
//! A field that is derived from other categories' data also lists them as *derived from*: **a wipe
//! of any category clears the fields it owns and the fields derived from it**, because a wipe
//! removes everything derived from the wiped data (`privacy-and-signals.md`). Nothing else changes.
//!
//! | category | tier | owns | also clears (derived from it) |
//! |---|---|---|---|
//! | [`Clock`](ContextCategory::Clock) | T1 | daypart, weekday type | activity |
//! | [`OutputDevice`](ContextCategory::OutputDevice) | T1 | device class | |
//! | [`ForegroundApp`](ContextCategory::ForegroundApp) | T2 | activity | |
//! | [`NetworkPlace`](ContextCategory::NetworkPlace) | T2 | place slot | activity |
//! | [`Weather`](ContextCategory::Weather) | T3 | weather condition, temperature band | |
//!
//! The activity guess is derived (`context-engine.md`, "Derived context"): `Focus`, `Gaming` and
//! `Call` from the app category, `OnTheMove` from the place, `Leisure` from the weekend or the
//! evening. It is owned by the foreground-app category (its sensitive values come from there) and
//! derived from the network place and the clock too, so wiping any of the three clears it.
//! Idle, fullscreen and power (T1) also feed it but are not a modeled category, so they clear
//! nothing; if one ever becomes a wipeable category it must be added to
//! [`ContextCategory::derived_fields`] for activity. Calendar (T2) has no `ContextKey` field. T0
//! has none either, so wiping T0 context is a no-op. Wiping a tier clears the fields of all its
//! categories.
//!
//! # Cost and locking
//!
//! A wipe holds the write lock of `signals.db` for its whole transaction, and the signal writer's
//! batches wait for it (5 s busy timeout) instead of failing. So the transaction is short: each
//! of the two tables is scanned a fixed number of times (once to check the stored keys, once to
//! rewrite them), whatever the number of distinct keys, and only rows that change are written.
//! `play.context_key` has no index and gets none for this (`storage-budget.md`). Measured with
//! 131 400 plays (3 years of the reference listener) and thousands of distinct keys: see the
//! `a_write_batch_during_a_wipe_of_the_reference_table_is_not_starved` test.
//!
//! # Not yet covered
//!
//! `context_snapshot.fields` (the quantized `ContextFields` blob) has no encoding yet, so this
//! wipe cannot remove a category's values from it. The ticket that defines the blob must extend
//! [`SignalWiper`] in the same transaction; until then only the keys are rewritten.
//!
//! `context_snapshot.hash` (a hash of the quantized fields, UNIQUE) is derived from the same
//! values: after a wipe a row's `key` no longer matches its `hash` and `fields`. A context hub
//! that dedupes against the database by hash must account for that; this belongs with the blob
//! work above.

use std::path::Path;

use rusqlite::{Connection, OptionalExtension, Transaction, TransactionBehavior};
use tinwave_core::Tier;
use tinwave_signals::{ContextField, ContextFieldSet};

use crate::connection;
use crate::migrate::migrate;
use crate::store::SIGNALS_MIGRATIONS;
use crate::DbError;

/// SQLite page cache of the wipe connection, in KiB: a wipe is rare.
const PAGE_CACHE_KIB: u32 = 256;

/// A group of context the listener can wipe: one context provider group, with the tier its data
/// belongs to. See the module docs for the fields each owns.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ContextCategory {
    /// Clock, daypart, weekday, holidays (T1).
    Clock,
    /// The output device (T1).
    OutputDevice,
    /// Foreground-app category, and the activity guess it drives (T2).
    ForegroundApp,
    /// The network place (T2).
    NetworkPlace,
    /// Weather (T3).
    Weather,
}

impl ContextCategory {
    /// Every category.
    pub const ALL: [Self; 5] = [
        Self::Clock,
        Self::OutputDevice,
        Self::ForegroundApp,
        Self::NetworkPlace,
        Self::Weather,
    ];

    /// The privacy tier of the category's data.
    #[must_use]
    pub const fn tier(self) -> Tier {
        match self {
            Self::Clock | Self::OutputDevice => Tier::T1,
            Self::ForegroundApp | Self::NetworkPlace => Tier::T2,
            Self::Weather => Tier::T3,
        }
    }

    /// The `ContextKey` fields a wipe of this category clears: the ones it
    /// [owns](Self::owned_fields) and the ones [derived from it](Self::derived_fields).
    #[must_use]
    pub const fn fields(self) -> ContextFieldSet {
        self.owned_fields().union(self.derived_fields())
    }

    /// The fields this category is the one owner of: its provider's values.
    #[must_use]
    pub const fn owned_fields(self) -> ContextFieldSet {
        match self {
            Self::Clock => ContextFieldSet::EMPTY
                .with(ContextField::Daypart)
                .with(ContextField::WeekdayType),
            Self::OutputDevice => ContextFieldSet::EMPTY.with(ContextField::DeviceClass),
            Self::ForegroundApp => ContextFieldSet::EMPTY.with(ContextField::Activity),
            Self::NetworkPlace => ContextFieldSet::EMPTY.with(ContextField::Place),
            Self::Weather => ContextFieldSet::EMPTY
                .with(ContextField::Condition)
                .with(ContextField::Temperature),
        }
    }

    /// Fields owned by another category but derived from this one's data, so a wipe of this
    /// category clears them too (see the module docs).
    #[must_use]
    pub const fn derived_fields(self) -> ContextFieldSet {
        match self {
            // `Leisure` is the guess for a weekend or an evening; `OnTheMove` uses the place.
            Self::Clock | Self::NetworkPlace => ContextFieldSet::EMPTY.with(ContextField::Activity),
            Self::OutputDevice | Self::ForegroundApp | Self::Weather => ContextFieldSet::EMPTY,
        }
    }

    /// The fields of every category in `tier`; empty for T0.
    #[must_use]
    pub const fn tier_fields(tier: Tier) -> ContextFieldSet {
        let mut fields = ContextFieldSet::EMPTY;
        let mut i = 0;
        while i < Self::ALL.len() {
            if Self::ALL[i].tier().number() == tier.number() {
                fields = fields.union(Self::ALL[i].fields());
            }
            i += 1;
        }
        fields
    }
}

/// What a wipe changed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct WipeReport {
    /// `play` rows whose `context_key` changed.
    pub plays_rewritten: usize,
    /// `context_snapshot` rows whose `key` changed.
    pub snapshots_rewritten: usize,
}

/// A connection to `signals.db` that wipes context data. Separate from the writer's connection
/// (like `PersistentSettings`): a wipe waits for the writer's current batch, never blocks it
/// for longer than its own transaction.
pub struct SignalWiper {
    conn: Connection,
}

impl std::fmt::Debug for SignalWiper {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SignalWiper").finish_non_exhaustive()
    }
}

impl SignalWiper {
    /// Opens `signals.db` at `path` (creating and migrating it if needed).
    ///
    /// # Errors
    /// [`DbError`] if the database cannot be opened or migrated.
    pub fn open(path: &Path) -> Result<Self, DbError> {
        let mut conn = connection::open(path, PAGE_CACHE_KIB)?;
        migrate(&mut conn, SIGNALS_MIGRATIONS)?;
        Ok(Self { conn })
    }

    /// Wipes the context of `categories`: the fields they own are set to unknown in every
    /// `context_snapshot.key` and `play.context_key`. One transaction: on error nothing changed.
    ///
    /// # Errors
    /// [`DbError::Sqlite`]; [`DbError::OutOfRange`] if a stored key is not a `u32` (the wipe
    /// stops and rolls back rather than leave a row it cannot clean).
    pub fn wipe_context(&mut self, categories: &[ContextCategory]) -> Result<WipeReport, DbError> {
        let fields = categories
            .iter()
            .fold(ContextFieldSet::EMPTY, |set, c| set.union(c.fields()));
        self.wipe_fields(fields)
    }

    /// Wipes the context of every category of `tier`. T0 has no context: nothing changes.
    ///
    /// # Errors
    /// As [`wipe_context`](Self::wipe_context).
    pub fn wipe_context_tier(&mut self, tier: Tier) -> Result<WipeReport, DbError> {
        self.wipe_fields(ContextCategory::tier_fields(tier))
    }

    fn wipe_fields(&mut self, fields: ContextFieldSet) -> Result<WipeReport, DbError> {
        if fields.is_empty() {
            return Ok(WipeReport::default());
        }
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(DbError::sqlite("starting the context wipe"))?;
        let plays_rewritten = rewrite_keys(&tx, "play", "context_key", fields)?;
        let snapshots_rewritten = rewrite_keys(&tx, "context_snapshot", "key", fields)?;
        tx.commit()
            .map_err(DbError::sqlite("committing the context wipe"))?;
        Ok(WipeReport {
            plays_rewritten,
            snapshots_rewritten,
        })
    }
}

/// Rewrites `column` of `table` with the `fields` cleared and returns how many rows changed.
///
/// Cost: one scan of the table to look for a value that is not a `u32`, and one `UPDATE` scan
/// that rewrites the rows that change, in the SQL below. Never one statement per distinct key:
/// `play.context_key` has no index (the storage budget does not pay for one), so a statement per
/// key would scan the table once per key. For the reference listener (131 400 plays in 3 years,
/// thousands of distinct keys) that held the write lock for many seconds and made the writer's
/// batches fail with `SQLITE_BUSY`.
///
/// The `UPDATE` is [`tinwave_signals::ContextKey::clear_raw`] in SQL, with `keep` the bits that stay and `region`
/// the field bits 0-23: the result is `c & keep`, or 0 when no field bit is left in it. A row
/// changes when it has a bit to clear, or when it is a non-zero key with no field bit at all (a
/// version nibble alone: `clear_raw` makes it the all-unknown 0), and a test pins both against
/// `clear_raw`. Rows that do not change are not written.
fn rewrite_keys(
    tx: &Transaction<'_>,
    table: &'static str,
    column: &'static str,
    fields: ContextFieldSet,
) -> Result<usize, DbError> {
    // A key that is not an integer in `0..=u32::MAX` cannot be cleaned bit by bit; stop and roll
    // back rather than report success with a row left behind. The check also guarantees that
    // the `UPDATE` below sees only valid `u32` values.
    let stray = tx
        .query_row(
            &format!(
                "SELECT {column} FROM {table} \
                 WHERE typeof({column}) != 'integer' OR {column} < 0 OR {column} > ?1 LIMIT 1"
            ),
            [i64::from(u32::MAX)],
            |row| {
                Ok(match row.get_ref(0)? {
                    rusqlite::types::ValueRef::Integer(n) => n.unsigned_abs(),
                    _ => u64::MAX,
                })
            },
        )
        .optional()
        .map_err(DbError::sqlite("checking the stored context keys"))?;
    if let Some(value) = stray {
        return Err(DbError::OutOfRange {
            what: "stored context key",
            value,
        });
    }
    let clear = i64::from(fields.bits());
    let keep = i64::from(u32::MAX) ^ clear;
    let region = i64::from(ContextFieldSet::ALL.bits());
    tx.execute(
        &format!(
            "UPDATE {table} SET {column} = \
                 CASE WHEN ({column} & ?1) & ?3 = 0 THEN 0 ELSE {column} & ?1 END \
             WHERE {column} != 0 AND (({column} & ?2) != 0 OR ({column} & ?3) = 0)"
        ),
        [keep, clear, region],
    )
    .map_err(DbError::sqlite("rewriting the context keys"))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The mapping is the one in the module docs and the privacy rules: T1 clock and device,
    /// T2 app and place, T3 weather, and every key field has exactly one owner.
    #[test]
    fn every_key_field_has_exactly_one_owning_category() {
        for field in ContextField::ALL {
            let owners: Vec<_> = ContextCategory::ALL
                .iter()
                .filter(|c| c.owned_fields().contains(field))
                .collect();
            assert_eq!(owners.len(), 1, "{field:?} is owned by {owners:?}");
        }
        // A derived field is never also the category's own: it has another owner.
        for category in ContextCategory::ALL {
            for field in ContextField::ALL {
                assert!(
                    !(category.owned_fields().contains(field)
                        && category.derived_fields().contains(field)),
                    "{category:?} lists {field:?} as owned and derived"
                );
            }
        }
        assert_eq!(ContextCategory::Weather.tier(), Tier::T3);
        assert_eq!(ContextCategory::NetworkPlace.tier(), Tier::T2);
        assert_eq!(ContextCategory::ForegroundApp.tier(), Tier::T2);
        assert_eq!(ContextCategory::Clock.tier(), Tier::T1);
        assert_eq!(ContextCategory::OutputDevice.tier(), Tier::T1);
    }

    /// Activity is owned by the foreground-app category and derived from the place and the clock:
    /// wiping any of the three clears it, and nothing else of the others (the device class stays).
    #[test]
    fn activity_is_cleared_by_the_categories_it_is_derived_from() {
        for category in [
            ContextCategory::ForegroundApp,
            ContextCategory::NetworkPlace,
            ContextCategory::Clock,
        ] {
            assert!(
                category.fields().contains(ContextField::Activity),
                "{category:?}"
            );
        }
        for category in [ContextCategory::OutputDevice, ContextCategory::Weather] {
            assert!(
                !category.fields().contains(ContextField::Activity),
                "{category:?}"
            );
        }
        for category in ContextCategory::ALL {
            assert_eq!(
                category.fields().contains(ContextField::DeviceClass),
                category == ContextCategory::OutputDevice,
                "{category:?}"
            );
        }
        assert_eq!(
            ContextCategory::NetworkPlace.fields(),
            ContextFieldSet::EMPTY
                .with(ContextField::Place)
                .with(ContextField::Activity)
        );
    }

    #[test]
    fn a_tier_clears_the_fields_of_its_categories() {
        assert!(ContextCategory::tier_fields(Tier::T0).is_empty());
        assert_eq!(
            ContextCategory::tier_fields(Tier::T2),
            ContextFieldSet::EMPTY
                .with(ContextField::Activity)
                .with(ContextField::Place)
        );
        assert_eq!(
            ContextCategory::tier_fields(Tier::T3),
            ContextCategory::Weather.fields()
        );
        let all = Tier::ALL.iter().fold(ContextFieldSet::EMPTY, |s, t| {
            s.union(ContextCategory::tier_fields(*t))
        });
        assert!(ContextField::ALL.iter().all(|f| all.contains(*f)));
    }
}
