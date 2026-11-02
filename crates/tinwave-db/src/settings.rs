//! Signal Center toggles that survive a restart.
//!
//! [`PersistentSettings`] owns the [`SignalSettings`] the bus checks before it constructs an
//! event, and keeps the one `signal_settings` row of `signals.db` (migration 0002) in step with it.
//!
//! - **Load before the bus accepts events.** [`PersistentSettings::open`] reads the row and
//!   builds the settings; hand [`settings`](PersistentSettings::settings) to `SignalBus::channel`.
//! - **Save on change.** [`set_category`](PersistentSettings::set_category),
//!   and [`set_tier`](PersistentSettings::set_tier) change the atomic mask first, so the toggle is in effect for the very next `emit_with`
//!   on every thread, then write the row.
//! - **No signal.** A toggle change writes settings, never an event (`design-system.md`
//!   `TwSignalRow`): turning collection off must not itself be collected.
//!
//! # First run, and a row that cannot be trusted
//!
//! No row and no data in the database is a first run: the documented defaults
//! (`privacy-and-signals.md`: every category on, T0 and T1 on, T2 and T3 off) are used and
//! stored; so is a database upgraded from before migration 0002 (open the settings first, see
//! [`PersistentSettings::open`]). A row that is missing from a database that already holds signals, or that does not
//! decode to a valid mask, is **not** silent: the defaults are used (so collection keeps working
//! as documented), the row is left as found until the user changes something, and
//! [`PersistentSettings::load_problem`] returns a [`SettingsProblem`] the Signal Center must show
//! as an error ("your settings could not be read, defaults are in use"). The same problem is
//! logged at `error`.
//!
//! A write that fails (disk full, locked database) returns the `DbError`: the toggle **is** in
//! effect now, but would be forgotten at the next start; the caller must tell the user.

use std::path::Path;
use std::sync::{Arc, Mutex, PoisonError};

use rusqlite::{params, Connection, OptionalExtension};
use tinwave_core::{Clock, Tier};
use tinwave_signals::{SignalCategory, SignalSettings};

use crate::connection;
use crate::migrate::migrate;
use crate::store::SIGNALS_MIGRATIONS;
use crate::DbError;

/// SQLite page cache of the settings connection, in KiB: one tiny row.
const PAGE_CACHE_KIB: u32 = 64;

/// The migration that creates `signal_settings`.
const SETTINGS_MIGRATION: u32 = 2;

const SELECT_MASK: &str = "SELECT mask FROM signal_settings WHERE id = 1";
const UPSERT: &str = "INSERT INTO signal_settings (id, mask, updated_ts) VALUES (1, ?1, ?2) \
    ON CONFLICT (id) DO UPDATE SET mask = excluded.mask, updated_ts = excluded.updated_ts";

/// Why the stored toggles could not be used. The defaults are in effect; show this to the user.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SettingsProblem {
    /// The database holds signals but no settings row: it was deleted or never written.
    MissingRow,
    /// The row exists but does not hold a valid mask.
    Corrupt {
        /// What was found: the stored value as text, or its SQL type.
        found: String,
    },
}

impl std::fmt::Display for SettingsProblem {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::MissingRow => write!(
                f,
                "the stored Signal Center settings are missing; the default settings are in use"
            ),
            Self::Corrupt { found } => write!(
                f,
                "the stored Signal Center settings are not valid ({found}); the default settings are in use"
            ),
        }
    }
}

/// How the toggles were obtained at startup.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SettingsLoad {
    /// A valid row was found.
    Restored,
    /// A new database: the defaults were stored.
    FirstRun,
    /// The row could not be used; the defaults are in effect and the row is unchanged.
    Defaulted(SettingsProblem),
}

/// The Signal Center toggles and their row in `signals.db`.
pub struct PersistentSettings {
    settings: Arc<SignalSettings>,
    conn: Mutex<Connection>,
    clock: Arc<dyn Clock>,
    load: SettingsLoad,
}

impl std::fmt::Debug for PersistentSettings {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PersistentSettings")
            .field("mask", &self.settings.mask())
            .field("load", &self.load)
            .finish_non_exhaustive()
    }
}

impl PersistentSettings {
    /// Opens `signals.db` at `path` (creating and migrating it if needed) and loads the toggles.
    /// Call it before the bus is created, and before anything else opens `signals.db`: if this
    /// call is the one that adds the settings table to a database that already holds signals
    /// (an upgrade), the missing row is a first run, not a [`SettingsProblem::MissingRow`].
    ///
    /// # Errors
    /// [`DbError`] if the database cannot be opened or read, or the first-run defaults cannot
    /// be stored. A row that is merely invalid is not an error: see [`SettingsLoad::Defaulted`].
    pub fn open(path: &Path, clock: impl Clock + 'static) -> Result<Self, DbError> {
        let mut conn = connection::open(path, PAGE_CACHE_KIB)?;
        let migrated = migrate(&mut conn, SIGNALS_MIGRATIONS)?;
        // Open this before anything else opens signals.db: when this call is the one that
        // created the settings table (migration 0002), an empty table on a database that already
        // holds signals is an upgrade, not a deleted row.
        let table_is_new = migrated.version >= SETTINGS_MIGRATION
            && migrated
                .version
                .saturating_sub(u32::try_from(migrated.applied).unwrap_or(u32::MAX))
                < SETTINGS_MIGRATION;
        let (settings, load) = Self::read(&conn, table_is_new)?;
        let this = Self {
            settings: Arc::new(settings),
            conn: Mutex::new(conn),
            clock: Arc::new(clock),
            load,
        };
        match &this.load {
            SettingsLoad::FirstRun => this.save()?,
            SettingsLoad::Defaulted(problem) => {
                tracing::error!(%problem, "Signal Center settings not restored");
            }
            SettingsLoad::Restored => {}
        }
        Ok(this)
    }

    fn read(
        conn: &Connection,
        table_is_new: bool,
    ) -> Result<(SignalSettings, SettingsLoad), DbError> {
        let stored = conn
            .query_row(SELECT_MASK, [], |row| {
                // Any SQL type: a text or NULL value is a corrupt row, not an I/O failure.
                row.get_ref(0).map(|v| match v {
                    rusqlite::types::ValueRef::Integer(n) => Ok(n),
                    other => Err(format!("a {} value", type_name(other.data_type()))),
                })
            })
            .optional()
            .map_err(DbError::sqlite("reading the Signal Center settings"))?;
        let corrupt = |found: String| {
            (
                SignalSettings::new(),
                SettingsLoad::Defaulted(SettingsProblem::Corrupt { found }),
            )
        };
        Ok(match stored {
            Some(Ok(n)) => match u32::try_from(n).map(SignalSettings::from_mask) {
                Ok(Ok(settings)) => (settings, SettingsLoad::Restored),
                _ => corrupt(format!("mask {n}")),
            },
            Some(Err(found)) => corrupt(found),
            None => {
                let has_data: bool = conn
                    .query_row(
                        "SELECT EXISTS (SELECT 1 FROM signal_event) \
                         OR EXISTS (SELECT 1 FROM ui_counter)",
                        [],
                        |row| row.get(0),
                    )
                    .map_err(DbError::sqlite("checking for stored signals"))?;
                if has_data && !table_is_new {
                    (
                        SignalSettings::new(),
                        SettingsLoad::Defaulted(SettingsProblem::MissingRow),
                    )
                } else {
                    (SignalSettings::new(), SettingsLoad::FirstRun)
                }
            }
        })
    }

    /// The toggles every producer checks. Give this to `SignalBus::channel`.
    #[must_use]
    pub fn settings(&self) -> &Arc<SignalSettings> {
        &self.settings
    }

    /// How the toggles were obtained at startup.
    #[must_use]
    pub fn load(&self) -> &SettingsLoad {
        &self.load
    }

    /// What the Signal Center must show as an error, if the stored settings could not be used.
    #[must_use]
    pub fn load_problem(&self) -> Option<&SettingsProblem> {
        match &self.load {
            SettingsLoad::Defaulted(problem) => Some(problem),
            SettingsLoad::Restored | SettingsLoad::FirstRun => None,
        }
    }

    /// Turns a category on or off: in effect now, then stored.
    ///
    /// # Errors
    /// [`DbError`] if the row could not be written. The toggle is in effect anyway; it would be
    /// lost at the next start, so tell the user.
    pub fn set_category(&self, category: SignalCategory, on: bool) -> Result<(), DbError> {
        self.settings.set_category(category, on);
        self.save()
    }

    /// Turns a tier on or off: in effect now, then stored.
    ///
    /// # Errors
    /// As [`set_category`](Self::set_category).
    pub fn set_tier(&self, tier: Tier, on: bool) -> Result<(), DbError> {
        self.settings.set_tier(tier, on);
        self.save()
    }

    /// Writes the current mask. The mask is read under the connection lock, so concurrent
    /// toggles leave the row equal to the last in-memory state.
    fn save(&self) -> Result<(), DbError> {
        let conn = self.conn.lock().unwrap_or_else(PoisonError::into_inner);
        conn.execute(
            UPSERT,
            params![i64::from(self.settings.mask()), self.clock.now().as_i64()],
        )
        .map_err(DbError::sqlite("storing the Signal Center settings"))?;
        Ok(())
    }
}

fn type_name(t: rusqlite::types::Type) -> &'static str {
    use rusqlite::types::Type;
    match t {
        Type::Null => "NULL",
        Type::Integer => "integer",
        Type::Real => "real",
        Type::Text => "text",
        Type::Blob => "blob",
    }
}
