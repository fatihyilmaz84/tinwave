//! Signal Center toggles that survive a restart.
//!
//! [`PersistentSettings`] owns the [`SignalSettings`] the bus checks before it constructs an
//! event, and keeps the one `signal_settings` row of `signals.db` (migration 0002) in step with it.
//!
//! - **Load before the bus accepts events.** [`PersistentSettings::open`] reads the row and
//!   builds the settings; hand [`settings`](PersistentSettings::settings) to `SignalBus::channel`.
//! - **Save on change.** [`set_category`](PersistentSettings::set_category)
//!   and [`set_tier`](PersistentSettings::set_tier) change the atomic mask first, so the toggle is
//!   in effect for the very next `emit_with` on every thread, then write the row.
//! - **Toggle only through here.** The `Arc<SignalSettings>` from
//!   [`settings`](PersistentSettings::settings) is for `SignalBus::channel` and for reading.
//!   `SignalSettings` has its own `set_category` and `set_tier` (the bus tests flip toggles
//!   without a database); on this instance they change what is collected but not the stored row,
//!   so the choice would be lost at the next start. Producers and the UI must toggle through
//!   `PersistentSettings`.
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
//! after which the next start is clean, and [`load_problem`](PersistentSettings::load_problem)
//! returns `None` from that save on, so the banner goes away in the same session.
//!
//! A write that fails (disk full, locked database) returns the `DbError`: the toggle **is** in
//! effect now, but would be forgotten at the next start; the caller must tell the user.

use std::path::Path;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

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
    /// How the toggles were obtained at startup. Never changes.
    load: SettingsLoad,
    /// The startup problem while it still applies: cleared by the first successful save, which
    /// replaces the row it came from.
    problem: Mutex<Option<SettingsProblem>>,
    /// Runs once, in the next save, after the atomic update and before the connection lock is
    /// requested (see `lock_conn`). Lets a test interleave a second toggle at that exact point.
    #[cfg(test)]
    before_lock: Mutex<Option<Box<dyn FnOnce() + Send>>>,
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
        let problem = match &load {
            SettingsLoad::FailedClosed(problem) => Some(problem.clone()),
            SettingsLoad::Restored => None,
        };
        Ok(Self {
            settings: Arc::new(settings),
            conn: Mutex::new(conn),
            clock: Arc::new(clock),
            load,
            problem: Mutex::new(problem),
            #[cfg(test)]
            before_lock: Mutex::new(None),
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

    /// The toggles every producer checks. Give this to `SignalBus::channel`; everyone else only
    /// reads it.
    ///
    /// Do not call `set_category` or `set_tier` on the returned settings: they take effect but are
    /// not stored (see the module docs). Toggle with [`set_category`](Self::set_category) and
    /// [`set_tier`](Self::set_tier) on `self`.
    #[must_use]
    pub fn settings(&self) -> &Arc<SignalSettings> {
        &self.settings
    }

    /// How the toggles were obtained at startup. This does not change when the user saves a
    /// toggle; to know whether the problem still applies, ask [`load_problem`](Self::load_problem).
    #[must_use]
    pub fn load(&self) -> &SettingsLoad {
        &self.load
    }

    /// What the Signal Center must show as an error right now: the startup problem with the
    /// stored settings, until the first successful save replaces the row it came from. `None`
    /// when the settings were restored, and from then on.
    ///
    /// A failed save does not clear it: the row is still the unusable one.
    #[must_use]
    pub fn load_problem(&self) -> Option<SettingsProblem> {
        self.problem
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
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
    /// toggles leave the row equal to the last in-memory state: the save that gets the lock last
    /// has seen every earlier atomic update. A successful write also clears the startup problem.
    fn save(&self) -> Result<(), DbError> {
        let conn = self.lock_conn();
        conn.execute(
            UPSERT,
            params![i64::from(self.settings.mask()), self.clock.now().as_i64()],
        )
        .map_err(DbError::sqlite("storing the Signal Center settings"))?;
        let cleared = self
            .problem
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take();
        if let Some(problem) = cleared {
            tracing::info!(%problem, "Signal Center settings saved; the stored row is usable again");
        }
        Ok(())
    }

    /// The connection, for a save. The mask must be read only after this returns.
    fn lock_conn(&self) -> MutexGuard<'_, Connection> {
        #[cfg(test)]
        {
            let hook = self
                .before_lock
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .take();
            if let Some(hook) = hook {
                hook();
            }
        }
        self.conn.lock().unwrap_or_else(PoisonError::into_inner)
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

#[cfg(test)]
mod tests {
    use std::sync::mpsc;

    use tinwave_core::{ManualClock, UtcMillis};

    use super::*;

    fn open_temp() -> (tempfile::TempDir, PersistentSettings) {
        let dir = tempfile::tempdir().unwrap();
        let settings = PersistentSettings::open(
            &dir.path().join("signals.db"),
            ManualClock::new(UtcMillis::new(1_793_577_600_000)),
        )
        .unwrap();
        (dir, settings)
    }

    fn stored(settings: &PersistentSettings) -> i64 {
        let conn = settings.conn.lock().unwrap();
        conn.query_row(SELECT_MASK, [], |row| row.get(0)).unwrap()
    }

    /// The upsert reads the mask under the connection lock.
    ///
    /// Toggle A is held after its atomic update, just before it asks for the lock. Toggle B then
    /// runs to completion and stores a mask that includes A's change. When A resumes it must store
    /// the current mask, not one it read earlier: otherwise A overwrites B's row with a mask that
    /// lacks B's toggle and the row differs from what is collected.
    #[test]
    fn the_mask_is_read_under_the_connection_lock() {
        let (_dir, settings) = open_temp();
        let (reached_tx, reached_rx) = mpsc::channel();
        let (go_tx, go_rx) = mpsc::channel::<()>();
        *settings.before_lock.lock().unwrap() = Some(Box::new(move || {
            reached_tx.send(()).unwrap();
            go_rx.recv().unwrap();
        }));

        std::thread::scope(|scope| {
            let a = scope.spawn(|| settings.set_category(SignalCategory::Volume, false));
            reached_rx.recv().unwrap();
            // A's category is already off in memory; its save has not started.
            assert!(!settings.settings().category_enabled(SignalCategory::Volume));

            settings.set_tier(Tier::T2, true).unwrap();
            assert_eq!(stored(&settings), i64::from(settings.settings().mask()));

            go_tx.send(()).unwrap();
            a.join().unwrap().unwrap();
        });

        let mask = settings.settings().mask();
        assert!(settings.settings().tier_enabled(Tier::T2), "B's tier is on");
        assert!(
            !settings.settings().category_enabled(SignalCategory::Volume),
            "A's category is off"
        );
        assert_eq!(
            stored(&settings),
            i64::from(mask),
            "the row equals the last in-memory state"
        );
    }

    #[test]
    fn the_hook_runs_once_and_only_for_the_next_save() {
        let (_dir, settings) = open_temp();
        let (tx, rx) = mpsc::channel();
        *settings.before_lock.lock().unwrap() = Some(Box::new(move || tx.send(()).unwrap()));
        settings.set_tier(Tier::T3, true).unwrap();
        settings.set_tier(Tier::T3, false).unwrap();
        assert_eq!(rx.try_iter().count(), 1);
    }
}
