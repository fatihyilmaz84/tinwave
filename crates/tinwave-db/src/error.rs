use thiserror::Error;

/// Errors from `tinwave-db`. Every SQLite failure keeps the `rusqlite::Error` as its
/// [`source`](std::error::Error::source), so callers can walk or downcast the chain.
#[derive(Debug, Error)]
pub enum DbError {
    /// SQLite reported an error while doing `context`.
    #[error("sqlite error while {context}")]
    Sqlite {
        /// What the store was doing.
        context: &'static str,
        /// The database error, unchanged.
        #[source]
        source: rusqlite::Error,
    },
    /// The database file could not be prepared (directory missing, file not a database, ...).
    #[error("cannot open the database: {reason}")]
    Open {
        /// Why.
        reason: String,
        /// The database error, if SQLite produced one.
        #[source]
        source: Option<rusqlite::Error>,
    },
    /// A connection setting did not take effect (for example WAL on a file system that cannot
    /// do it, or `auto_vacuum` on a database created without it).
    #[error("pragma {pragma} is {actual}, expected {expected}")]
    Pragma {
        /// The pragma name.
        pragma: &'static str,
        /// What SQLite reports.
        actual: String,
        /// What the database rules require.
        expected: &'static str,
    },
    /// The migration list handed to the runner is not numbered `1, 2, 3, ...` without gaps.
    #[error("migration list is not contiguous: expected version {expected}, found {found}")]
    MigrationOrder {
        /// The version that should come next.
        expected: u32,
        /// The version found.
        found: u32,
    },
    /// The database was migrated by a newer build than this one.
    #[error("database is at migration {found}, newer than the {known} this build knows")]
    DatabaseTooNew {
        /// Highest version recorded in the database.
        found: u32,
        /// Highest version this build ships.
        known: u32,
    },
    /// An applied migration no longer matches the SQL shipped with this build. Migrations are
    /// never edited after release.
    #[error("migration {version} ({name}) was edited after it was applied")]
    MigrationChanged {
        /// Version of the migration.
        version: u32,
        /// Its name.
        name: &'static str,
    },
    /// The writer thread could not be started.
    #[error("cannot start the signal writer thread")]
    Thread(#[source] std::io::Error),
    /// The writer thread panicked; whatever it still held in memory is lost.
    #[error("the signal writer thread panicked")]
    WriterPanicked,
    /// A value does not fit the SQLite integer it must be stored in.
    #[error("{what} = {value} does not fit a SQLite integer")]
    OutOfRange {
        /// Which field.
        what: &'static str,
        /// The value.
        value: u64,
    },
}

impl DbError {
    pub(crate) fn sqlite(context: &'static str) -> impl FnOnce(rusqlite::Error) -> Self {
        move |source| Self::Sqlite { context, source }
    }

    /// The error and its whole `source` chain on one line, `a: b: c`.
    ///
    /// This is what crosses the boundary into [`tinwave_signals::SignalsError::Sink`], whose
    /// `reason` is a string. Inside this crate the typed error is kept.
    #[must_use]
    pub fn chain(&self) -> String {
        let mut text = self.to_string();
        let mut next = std::error::Error::source(self);
        while let Some(cause) = next {
            text.push_str(": ");
            text.push_str(&cause.to_string());
            next = cause.source();
        }
        text
    }
}
