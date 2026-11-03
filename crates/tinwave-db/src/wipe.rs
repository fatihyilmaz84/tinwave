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
//! # Which fields a category owns
//!
//! A [`ContextCategory`] is a provider group of the context engine
//! (`docs/guides/context-engine.md`) with the tier of `signal-catalog.md` section 7. Wiping it
//! sets the `ContextKey` fields below to unknown, and nothing else:
//!
//! | category | tier | fields |
//! |---|---|---|
//! | [`Clock`](ContextCategory::Clock) | T1 | daypart, weekday type |
//! | [`OutputDevice`](ContextCategory::OutputDevice) | T1 | device class |
//! | [`ForegroundApp`](ContextCategory::ForegroundApp) | T2 | activity |
//! | [`NetworkPlace`](ContextCategory::NetworkPlace) | T2 | place slot |
//! | [`Weather`](ContextCategory::Weather) | T3 | weather condition, temperature band |
//!
//! The activity guess is derived from several providers (app category, idle, fullscreen, power,
//! place, clock). It is owned by the foreground-app category, the T2 input its sensitive values
//! (`Focus`, `Gaming`, `Call`) come from; wiping a different category does not clear it, and
//! wiping the tier T2 does. Calendar (T2) has no `ContextKey` field. T0 has none either, so
//! wiping T0 context is a no-op.
//!
//! # Not yet covered
//!
//! `context_snapshot.fields` (the quantized `ContextFields` blob) has no encoding yet, so this
//! wipe cannot remove a category's values from it. The ticket that defines the blob must extend
//! [`SignalWiper`] in the same transaction; until then only the keys are rewritten.

use std::path::Path;

use rusqlite::{Connection, Transaction, TransactionBehavior};
use tinwave_core::Tier;
use tinwave_signals::{ContextField, ContextFieldSet, ContextKey};

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

    /// The `ContextKey` fields a wipe of this category clears.
    #[must_use]
    pub const fn fields(self) -> ContextFieldSet {
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

/// Rewrites `column` of `table` with the `fields` cleared, one `UPDATE` per distinct stored key
/// that changes, and returns how many rows changed. There are few distinct keys (a key is a
/// bucket, not a moment), so this is a handful of statements, not one per row.
///
/// The keys to rewrite are all read before the first update. Clearing is idempotent, so a key
/// that an earlier update produced is its own result and never changes again.
fn rewrite_keys(
    tx: &Transaction<'_>,
    table: &'static str,
    column: &'static str,
    fields: ContextFieldSet,
) -> Result<usize, DbError> {
    let keys: Vec<i64> = {
        let mut stmt = tx
            .prepare(&format!(
                "SELECT DISTINCT {column} FROM {table} WHERE {column} != 0 ORDER BY {column}"
            ))
            .map_err(DbError::sqlite("listing the stored context keys"))?;
        let rows = stmt
            .query_map([], |row| row.get(0))
            .map_err(DbError::sqlite("listing the stored context keys"))?;
        rows.collect::<rusqlite::Result<_>>()
            .map_err(DbError::sqlite("listing the stored context keys"))?
    };
    let mut update = tx
        .prepare(&format!(
            "UPDATE {table} SET {column} = ?1 WHERE {column} = ?2"
        ))
        .map_err(DbError::sqlite("preparing the context key rewrite"))?;
    let mut rewritten = 0;
    for old in keys {
        let raw = u32::try_from(old).map_err(|_| DbError::OutOfRange {
            what: "stored context key",
            value: old.unsigned_abs(),
        })?;
        let new = ContextKey::clear_raw(raw, fields);
        if new != raw {
            rewritten += update
                .execute([i64::from(new), old])
                .map_err(DbError::sqlite("rewriting a context key"))?;
        }
    }
    Ok(rewritten)
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
                .filter(|c| c.fields().contains(field))
                .collect();
            assert_eq!(owners.len(), 1, "{field:?} is owned by {owners:?}");
        }
        assert_eq!(ContextCategory::Weather.tier(), Tier::T3);
        assert_eq!(ContextCategory::NetworkPlace.tier(), Tier::T2);
        assert_eq!(ContextCategory::ForegroundApp.tier(), Tier::T2);
        assert_eq!(ContextCategory::Clock.tier(), Tier::T1);
        assert_eq!(ContextCategory::OutputDevice.tier(), Tier::T1);
    }

    #[test]
    fn a_tier_owns_the_fields_of_its_categories() {
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
