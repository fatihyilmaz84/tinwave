//! Opening a database file with the settings of `docs/rules/database.md`.

use std::path::Path;
use std::time::Duration;

use rusqlite::Connection;

use crate::DbError;

/// `PRAGMA auto_vacuum` value for INCREMENTAL.
const AUTO_VACUUM_INCREMENTAL: i64 = 2;
/// `journal_size_limit` from `docs/rules/database.md`: 16 MiB.
const JOURNAL_SIZE_LIMIT: i64 = 16_777_216;
/// How long a statement waits for a lock held by another connection before failing.
pub(crate) const BUSY_TIMEOUT: Duration = Duration::from_secs(5);

/// Opens (creating it if needed) the database at `path` and applies the connection settings:
/// 4 KiB pages, `auto_vacuum=INCREMENTAL`, WAL, `synchronous=NORMAL`, `foreign_keys=ON`,
/// `journal_size_limit=16 MiB`, automatic checkpoints off (the store checkpoints when idle).
///
/// `page_size` and `auto_vacuum` only take effect on a database that has no tables yet, so they
/// are set first. They are checked afterwards: a database that was created without
/// `auto_vacuum=INCREMENTAL` is an error, not something to run with silently.
///
/// `page_cache_kib` is the SQLite page cache of the connection (SQLite defaults to 2 MiB).
///
/// # Errors
/// [`DbError::Open`] if the file cannot be opened, [`DbError::Pragma`] if a setting did not
/// take effect, [`DbError::Sqlite`] for other failures.
pub(crate) fn open(path: &Path, page_cache_kib: u32) -> Result<Connection, DbError> {
    let conn = Connection::open(path).map_err(|source| DbError::Open {
        reason: "SQLite refused to open the file".to_owned(),
        source: Some(source),
    })?;
    conn.busy_timeout(BUSY_TIMEOUT)
        .map_err(DbError::sqlite("setting the busy timeout"))?;
    conn.pragma_update(None, "page_size", 4096)
        .map_err(DbError::sqlite("setting page_size"))?;
    conn.pragma_update(None, "auto_vacuum", "INCREMENTAL")
        .map_err(DbError::sqlite("setting auto_vacuum"))?;
    let mode: String = conn
        .pragma_update_and_check(None, "journal_mode", "WAL", |row| row.get(0))
        .map_err(DbError::sqlite("setting journal_mode"))?;
    if !mode.eq_ignore_ascii_case("wal") {
        return Err(DbError::Pragma {
            pragma: "journal_mode",
            actual: mode,
            expected: "wal",
        });
    }
    conn.pragma_update(None, "synchronous", "NORMAL")
        .map_err(DbError::sqlite("setting synchronous"))?;
    conn.pragma_update(None, "foreign_keys", true)
        .map_err(DbError::sqlite("setting foreign_keys"))?;
    conn.pragma_update(None, "journal_size_limit", JOURNAL_SIZE_LIMIT)
        .map_err(DbError::sqlite("setting journal_size_limit"))?;
    // Checkpoints fsync and take 100-800 ms on Windows; the writer runs them itself when it is
    // idle (`SignalStore::checkpoint_if_due`) instead of inside a commit.
    conn.pragma_update(None, "wal_autocheckpoint", 0)
        .map_err(DbError::sqlite("setting wal_autocheckpoint"))?;
    conn.pragma_update(None, "cache_size", -i64::from(page_cache_kib))
        .map_err(DbError::sqlite("setting cache_size"))?;
    let auto_vacuum: i64 = conn
        .pragma_query_value(None, "auto_vacuum", |row| row.get(0))
        .map_err(DbError::sqlite("reading auto_vacuum"))?;
    if auto_vacuum != AUTO_VACUUM_INCREMENTAL {
        return Err(DbError::Pragma {
            pragma: "auto_vacuum",
            actual: auto_vacuum.to_string(),
            expected: "2 (incremental)",
        });
    }
    Ok(conn)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pragma(conn: &Connection, name: &str) -> i64 {
        conn.pragma_query_value(None, name, |row| row.get(0))
            .unwrap()
    }

    #[test]
    fn a_new_database_gets_every_pragma_of_database_md() {
        let dir = tempfile::tempdir().unwrap();
        let conn = open(&dir.path().join("signals.db"), 256).unwrap();
        let mode: String = conn
            .pragma_query_value(None, "journal_mode", |row| row.get(0))
            .unwrap();
        assert_eq!(mode, "wal");
        assert_eq!(pragma(&conn, "synchronous"), 1, "NORMAL");
        assert_eq!(pragma(&conn, "foreign_keys"), 1);
        assert_eq!(pragma(&conn, "auto_vacuum"), 2, "INCREMENTAL");
        assert_eq!(pragma(&conn, "journal_size_limit"), 16_777_216);
        assert_eq!(pragma(&conn, "page_size"), 4096);
        assert_eq!(pragma(&conn, "wal_autocheckpoint"), 0);
        assert_eq!(pragma(&conn, "cache_size"), -256);
    }

    #[test]
    fn a_database_created_without_incremental_auto_vacuum_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("old.db");
        {
            let plain = Connection::open(&path).unwrap();
            plain
                .execute_batch("CREATE TABLE t (x INTEGER); INSERT INTO t VALUES (1);")
                .unwrap();
        }
        let error = open(&path, 256).unwrap_err();
        assert!(
            matches!(
                error,
                DbError::Pragma {
                    pragma: "auto_vacuum",
                    ..
                }
            ),
            "{error}"
        );
    }
}
