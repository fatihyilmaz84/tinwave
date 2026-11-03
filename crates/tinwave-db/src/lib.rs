//! Tinwave SQLite layer. The only crate that writes SQL (`docs/rules/architecture.md`).
//!
//! Today it holds the migration runner and `signals.db`:
//!
//! - [`migrate`]: numbered SQL migrations, one transaction each, with a version table and a
//!   checksum tripwire against edited migrations.
//! - [`SignalStore`]: the writer connection of `signals.db` (pragmas, schema, dictionaries
//!   behind a bounded LRU) and the [`SignalSink`](tinwave_signals::SignalSink) implementation:
//!   one transaction per batch of events and counter rows, plus the `play` row of every play that
//!   ends in the batch (`Completed` or `Skipped`, paired with its `PlayStarted` per device).
//! - [`PersistentSettings`]: the Signal Center toggles (`SignalSettings`) and their row in
//!   `signals.db`; loaded before the bus exists, saved on every change.
//! - [`SignalWiper`]: the user wipes of `signals.db`, each one transaction: a context category
//!   (clears its fields from `context_snapshot.key` and `play.context_key`), a signal category
//!   (its events and counters, and what is derived from them), and everything.
//! - [`SignalWriter`]: the thread that drains a [`SignalDrain`](tinwave_signals::SignalDrain)
//!   into the store every 2 s or 256 events, at once on a play end, and on shutdown.
//!
//! # Wiring
//!
//! ```text
//! let path = dir.join("signals.db");
//! let store = SignalStore::open(&path)?;
//! let toggles = PersistentSettings::open(&path, clock.clone())?;  // before the bus exists
//! // Unusable stored settings fail closed (every toggle off): show why in the Signal Center.
//! if let Some(problem) = toggles.load_problem() { /* show it */ }
//! let mut cfg = BusConfig::new(device);
//! cfg.first_seq = store.first_seq(device)?;          // never reuse a seq
//! let (bus, drain) = SignalBus::channel(cfg, clock, toggles.settings().clone())?;
//! // a Signal Center switch: in effect at once, stored, never an event
//! toggles.set_category(SignalCategory::Browsing, false)?;
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
mod play;
mod settings;
mod signal_wipe;
mod store;
mod wipe;
mod writer;

pub use error::DbError;
pub use migrate::{migrate, Migration, MigrationReport};
pub use play::{EndReason, PlayFlags};
pub use settings::{PersistentSettings, SettingsLoad, SettingsProblem};
pub use signal_wipe::SignalWipeReport;
pub use store::{
    ends_a_play, BatchReport, CheckpointPolicy, SessionTz, SignalStore, StoreConfig,
    SIGNALS_MIGRATIONS,
};
pub use wipe::{ContextCategory, SignalWiper, WipeReport};
pub use writer::{SignalWriter, WriterConfig, WriterStats};
