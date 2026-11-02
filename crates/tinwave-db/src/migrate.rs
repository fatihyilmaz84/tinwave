//! The migration runner: numbered SQL files, each applied in its own transaction, recorded in
//! a version table.
//!
//! Migrations are never edited after release (`docs/rules/database.md`). The runner enforces it:
//! it stores a checksum of every applied migration and refuses to start when the SQL shipped
//! with this build differs, or when the database is newer than the build.

use rusqlite::{params, Connection};

use crate::DbError;

/// One numbered migration.
#[derive(Debug, Clone, Copy)]
pub struct Migration {
    /// 1, 2, 3, ... without gaps.
    pub version: u32,
    /// Short name, from the file name.
    pub name: &'static str,
    /// The SQL, one or more statements.
    pub sql: &'static str,
}

/// What [`migrate`] did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MigrationReport {
    /// Migrations applied by this call.
    pub applied: usize,
    /// Version the database is at afterwards (0 when the list is empty).
    pub version: u32,
}

const CREATE_VERSION_TABLE: &str = "CREATE TABLE IF NOT EXISTS schema_migration (
    version  INTEGER PRIMARY KEY,
    name     TEXT NOT NULL,
    checksum INTEGER NOT NULL
)";

/// FNV-1a over the SQL text, stored as the migration's checksum. Not a security measure, only
/// a tripwire for edited migrations.
fn checksum(sql: &str) -> i64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in sql.bytes() {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0100_0000_01b3);
    }
    i64::from_ne_bytes(hash.to_ne_bytes())
}

/// Applies every migration the database has not seen yet, oldest first. Idempotent: a second
/// call on an up-to-date database changes nothing.
///
/// # Errors
/// - [`DbError::MigrationOrder`] if `migrations` is not numbered `1..=n`.
/// - [`DbError::DatabaseTooNew`] if the database has migrations this list lacks.
/// - [`DbError::MigrationChanged`] if an applied migration's SQL changed.
/// - [`DbError::Sqlite`] if a migration fails; it is rolled back and earlier ones stay applied.
pub fn migrate(
    conn: &mut Connection,
    migrations: &[Migration],
) -> Result<MigrationReport, DbError> {
    let mut expected = 1;
    for m in migrations {
        if m.version != expected {
            return Err(DbError::MigrationOrder {
                expected,
                found: m.version,
            });
        }
        expected += 1;
    }
    conn.execute_batch(CREATE_VERSION_TABLE)
        .map_err(DbError::sqlite("creating the migration table"))?;

    let applied: Vec<(u32, i64)> = {
        let reading = DbError::sqlite("reading applied migrations");
        let mut stmt = conn
            .prepare("SELECT version, checksum FROM schema_migration ORDER BY version")
            .map_err(reading)?;
        let rows = stmt
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
            .map_err(DbError::sqlite("reading applied migrations"))?;
        rows.collect::<Result<_, _>>()
            .map_err(DbError::sqlite("reading applied migrations"))?
    };
    let known = u32::try_from(migrations.len()).unwrap_or(u32::MAX);
    for &(version, stored) in &applied {
        let index = usize::try_from(version.saturating_sub(1)).unwrap_or(usize::MAX);
        let Some(m) = migrations.get(index) else {
            return Err(DbError::DatabaseTooNew {
                found: version,
                known,
            });
        };
        if checksum(m.sql) != stored {
            return Err(DbError::MigrationChanged {
                version,
                name: m.name,
            });
        }
    }

    let done = applied.len().min(migrations.len());
    for m in &migrations[done..] {
        let tx = conn
            .transaction()
            .map_err(DbError::sqlite("starting a migration"))?;
        tx.execute_batch(m.sql)
            .map_err(DbError::sqlite("applying a migration"))?;
        tx.execute(
            "INSERT INTO schema_migration (version, name, checksum) VALUES (?1, ?2, ?3)",
            params![m.version, m.name, checksum(m.sql)],
        )
        .map_err(DbError::sqlite("recording a migration"))?;
        tx.commit()
            .map_err(DbError::sqlite("committing a migration"))?;
    }
    Ok(MigrationReport {
        applied: migrations.len() - done,
        version: known,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const A: Migration = Migration {
        version: 1,
        name: "a",
        sql: "CREATE TABLE a (x INTEGER);",
    };
    const B: Migration = Migration {
        version: 2,
        name: "b",
        sql: "CREATE TABLE b (y INTEGER);",
    };

    fn conn() -> Connection {
        Connection::open_in_memory().unwrap()
    }

    #[test]
    fn applies_in_order_and_is_idempotent() {
        let mut c = conn();
        assert_eq!(
            migrate(&mut c, &[A, B]).unwrap(),
            MigrationReport {
                applied: 2,
                version: 2
            }
        );
        assert_eq!(
            migrate(&mut c, &[A, B]).unwrap(),
            MigrationReport {
                applied: 0,
                version: 2
            }
        );
        c.execute("INSERT INTO b VALUES (1)", []).unwrap();
    }

    #[test]
    fn a_new_migration_is_applied_on_top() {
        let mut c = conn();
        migrate(&mut c, &[A]).unwrap();
        let report = migrate(&mut c, &[A, B]).unwrap();
        assert_eq!((report.applied, report.version), (1, 2));
    }

    #[test]
    fn a_failing_migration_rolls_back_and_keeps_earlier_ones() {
        let mut c = conn();
        let bad = Migration {
            version: 2,
            name: "bad",
            sql: "CREATE TABLE half (x); THIS IS NOT SQL;",
        };
        assert!(matches!(
            migrate(&mut c, &[A, bad]),
            Err(DbError::Sqlite { .. })
        ));
        let tables: i64 = c
            .query_row(
                "SELECT count(*) FROM sqlite_master WHERE name IN ('a', 'half')",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(tables, 1, "a stays, half is rolled back");
        assert_eq!(migrate(&mut c, &[A, B]).unwrap().applied, 1);
    }

    #[test]
    fn rejects_gaps_newer_databases_and_edited_migrations() {
        let mut c = conn();
        let gap = Migration { version: 3, ..B };
        assert!(matches!(
            migrate(&mut c, &[A, gap]),
            Err(DbError::MigrationOrder {
                expected: 2,
                found: 3
            })
        ));
        migrate(&mut c, &[A, B]).unwrap();
        assert!(matches!(
            migrate(&mut c, &[A]),
            Err(DbError::DatabaseTooNew { found: 2, known: 1 })
        ));
        let edited = Migration {
            sql: "CREATE TABLE a (x INTEGER, z INTEGER);",
            ..A
        };
        assert!(matches!(
            migrate(&mut c, &[edited, B]),
            Err(DbError::MigrationChanged {
                version: 1,
                name: "a"
            })
        ));
    }
}
