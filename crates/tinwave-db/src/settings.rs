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
//! Migration 0002 creates the table with the documented defaults in its one row
//! (`privacy-and-signals.md`: every category on, T0 and T1 on, T2 and T3 off, mask 895, pinned to
//! `SignalSettings::new()` by a test), so a new database and an upgraded one start with them and
//! there is no first-run special case: a missing row is always a fault.
//!
//! A row that is missing or does not decode to a valid mask **fails closed**: the toggles start
//! as `SignalSettings::all_off()`, because collecting data a user may have switched off breaks the
//! product's promise, while losing some signals until they act does not. It is not silent:
//! [`PersistentSettings::load_problem`] returns a [`SettingsProblem`] the Signal Center must show
//! as an error ("collection is paused until you review the settings"), and the same problem is
//! logged at `error`. The row is left as found; the first toggle the user saves replaces it,
//! after which the next start is clean.
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

const SELECT_MASK: &str = "SELECT mask FROM signal_settings WHERE id = 1";
const UPSERT: &str = "INSERT INTO signal_settings (id, mask, updated_ts) VALUES (1, ?1, ?2) \
    ON CONFLICT (id) DO UPDATE SET mask = excluded.mask, updated_ts = excluded.updated_ts";

/// Why the stored toggles could not be used. Collection is paused (every toggle off); show this
/// to the user.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SettingsProblem {
    /// There is no settings row: migration 0002 creates it, so it was deleted.
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
                "the stored Signal Center settings are missing; collection is paused until you review the settings"
            ),
            Self::Corrupt { found } => write!(
                f,
                "the stored Signal Center settings are not valid ({found}); collection is paused until you review the settings"
            ),
        }
    }
}

/// How the toggles were obtained at startup.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SettingsLoad {
    /// A valid row was found (on a new database: the defaults the migration inserted).
    Restored,
    /// The row could not be used; every toggle is off and the row is unchanged.
    FailedClosed(SettingsProblem),
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
    /// Call it before the bus is created.
    ///
    /// # Errors
    /// [`DbError`] if the database cannot be opened or read. A row that is missing or invalid is
    /// not an error: see [`SettingsLoad::FailedClosed`].
    pub fn open(path: &Path, clock: impl Clock + 'static) -> Result<Self, DbError> {
        let mut conn = connection::open(path, PAGE_CACHE_KIB)?;
        migrate(&mut conn, SIGNALS_MIGRATIONS)?;
        let (settings, load) = Self::read(&conn)?;
        if let SettingsLoad::FailedClosed(problem) = &load {
            tracing::error!(%problem, "Signal Center settings not restored; collection is paused");
        }
        Ok(Self {
            settings: Arc::new(settings),
            conn: Mutex::new(conn),
            clock: Arc::new(clock),
            load,
        })
    }

    fn read(conn: &Connection) -> Result<(SignalSettings, SettingsLoad), DbError> {
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
        let closed = |problem: SettingsProblem| {
            (
                SignalSettings::all_off(),
                SettingsLoad::FailedClosed(problem),
            )
        };
        Ok(match stored {
            Some(Ok(n)) => match u32::try_from(n).map(SignalSettings::from_mask) {
                Ok(Ok(settings)) => (settings, SettingsLoad::Restored),
                _ => closed(SettingsProblem::Corrupt {
                    found: format!("mask {n}"),
                }),
            },
            Some(Err(found)) => closed(SettingsProblem::Corrupt { found }),
            None => closed(SettingsProblem::MissingRow),
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
            SettingsLoad::FailedClosed(problem) => Some(problem),
            SettingsLoad::Restored => None,
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
