//! Tinwave SQLite layer. The only crate that writes SQL (`docs/rules/architecture.md`).
//!
//! Today it holds the migration runner and `signals.db`:
//!
//! - [`migrate`]: numbered SQL migrations, one transaction each, with a version table and a
//!   checksum tripwire against edited migrations.
//! - [`SignalStore`]: the writer connection of `signals.db` (pragmas, schema, dictionaries
//!   behind a bounded LRU) and the [`SignalSink`](tinwave_signals::SignalSink) implementation:
//!   one transaction per batch of events and counter rows.
//! - [`SignalWriter`]: the thread that drains a [`SignalDrain`](tinwave_signals::SignalDrain)
//!   into the store every 2 s or 256 events, at once on a play end, and on shutdown.
//!
//! # Wiring
//!
//! ```text
//! let store = SignalStore::open(&dir.join("signals.db"))?;
//! let mut cfg = BusConfig::new(device);
//! cfg.first_seq = store.first_seq(device)?;          // never reuse a seq
//! let (bus, drain) = SignalBus::channel(cfg, clock, settings)?;
//! let writer = SignalWriter::spawn(store, drain, WriterConfig::default())?;
//! // at session start, with the same offset:
//! bus.set_tz_offset(tz);  writer.set_tz_offset(tz);
//! // at quit:
//! writer.shutdown()?;
//! ```

#![forbid(unsafe_code)]

mod connection;
mod dict;
mod error;
mod lru;
mod migrate;
mod store;
mod writer;

pub use error::DbError;
pub use migrate::{migrate, Migration, MigrationReport};
pub use store::{
    ends_a_play, BatchReport, CheckpointPolicy, SessionTz, SignalStore, StoreConfig,
    SIGNALS_MIGRATIONS,
};
pub use writer::{SignalWriter, WriterConfig, WriterStats};
