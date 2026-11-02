//! Dictionary lookups for one write transaction.
//!
//! Every UUID is stored once in a dictionary table and referenced by a small integer
//! everywhere else (`docs/rules/storage-budget.md` section 3). A [`Resolver`] answers
//! "which integer is this UUID?" from, in order, the bounded LRU, the rows it inserted earlier in
//! the same transaction, and the database; and inserts the dictionary row when it is new.
//!
//! **Nothing is added to the LRU while the transaction is open.** A dictionary row inserted
//! inside a transaction that then rolls back does not exist, and its integer may be handed to a
//! different UUID next time: a cache that remembered it would corrupt every later batch. New
//! entries are kept in `fresh` and published with [`Resolver::publish`] after the commit.

use std::collections::HashMap;

use rusqlite::{params, Connection, OptionalExtension};
use tinwave_core::SessionId;

use crate::lru::Lru;
use crate::DbError;

/// Which dictionary a UUID lives in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum Table {
    Device,
    Recording,
    Source,
    /// Albums, artists and playlists; rows carry their `EntityKind` code.
    Entity,
}

impl Table {
    const fn select_sql(self) -> &'static str {
        match self {
            Self::Device => "SELECT id FROM dict_device WHERE uuid = ?1",
            Self::Recording => "SELECT id FROM dict_recording WHERE uuid = ?1",
            Self::Source => "SELECT id FROM dict_source WHERE uuid = ?1",
            Self::Entity => "SELECT id FROM dict_entity WHERE uuid = ?1",
        }
    }

    const fn insert_sql(self) -> &'static str {
        match self {
            Self::Device => "INSERT INTO dict_device (uuid) VALUES (?1)",
            Self::Recording => "INSERT INTO dict_recording (uuid) VALUES (?1)",
            Self::Source => "INSERT INTO dict_source (uuid) VALUES (?1)",
            Self::Entity => "INSERT INTO dict_entity (uuid, kind) VALUES (?1, ?2)",
        }
    }
}

pub(crate) type DictKey = (Table, [u8; 16]);

/// The caches that outlive a transaction.
pub(crate) struct DictCaches {
    pub(crate) dict: Lru<DictKey, i64>,
    pub(crate) sessions: Lru<SessionId, i64>,
}

impl DictCaches {
    pub(crate) fn new(dict_capacity: usize, session_capacity: usize) -> Self {
        Self {
            dict: Lru::new(dict_capacity),
            sessions: Lru::new(session_capacity),
        }
    }
}

/// Resolves UUIDs inside one transaction. See the module docs for why `fresh` exists.
pub(crate) struct Resolver<'a> {
    caches: &'a mut DictCaches,
    fresh: HashMap<DictKey, i64>,
    fresh_sessions: HashMap<SessionId, i64>,
}

impl<'a> Resolver<'a> {
    pub(crate) fn new(caches: &'a mut DictCaches) -> Self {
        Self {
            caches,
            fresh: HashMap::new(),
            fresh_sessions: HashMap::new(),
        }
    }

    /// The integer for `uuid` in `table`, inserting the dictionary row if it is new.
    /// `entity_kind` is the `EntityKind` code stored with a [`Table::Entity`] row.
    pub(crate) fn resolve(
        &mut self,
        conn: &Connection,
        table: Table,
        uuid: &[u8; 16],
        entity_kind: u8,
    ) -> Result<i64, DbError> {
        let key = (table, *uuid);
        if let Some(id) = self.caches.dict.get(&key) {
            return Ok(id);
        }
        if let Some(&id) = self.fresh.get(&key) {
            return Ok(id);
        }
        let mut select = conn
            .prepare_cached(table.select_sql())
            .map_err(DbError::sqlite("preparing a dictionary lookup"))?;
        let found: Option<i64> = select
            .query_row([&uuid[..]], |row| row.get(0))
            .optional()
            .map_err(DbError::sqlite("looking up a dictionary entry"))?;
        let id = if let Some(id) = found {
            id
        } else {
            let mut insert = conn
                .prepare_cached(table.insert_sql())
                .map_err(DbError::sqlite("preparing a dictionary insert"))?;
            let inserted = if table == Table::Entity {
                insert.execute(params![&uuid[..], entity_kind])
            } else {
                insert.execute(params![&uuid[..]])
            };
            inserted.map_err(DbError::sqlite("inserting a dictionary entry"))?;
            conn.last_insert_rowid()
        };
        self.fresh.insert(key, id);
        Ok(id)
    }

    /// The cached integer of a session, if the writer already created its row.
    pub(crate) fn known_session(&mut self, session: SessionId) -> Option<i64> {
        self.caches
            .sessions
            .get(&session)
            .or_else(|| self.fresh_sessions.get(&session).copied())
    }

    /// Remembers a session row inserted in this transaction.
    pub(crate) fn new_session(&mut self, session: SessionId, id: i64) {
        self.fresh_sessions.insert(session, id);
    }

    /// Call after the transaction committed: moves the new entries into the LRUs.
    pub(crate) fn publish(self) {
        for (key, id) in self.fresh {
            self.caches.dict.insert(key, id);
        }
        for (session, id) in self.fresh_sessions {
            self.caches.sessions.insert(session, id);
        }
    }
}
