HANDOFF T-0006 (round 1)
status: partial
branch: t/T-0006-signals-db-writer   commit: d806989 (includes a merge of main at a5b7e1b; Cargo.lock regenerated)
summary: New crate `tinwave-db`: migration runner (numbered SQL, transactional, version table, checksum tripwire), signals.db migration 0001 exactly as in database.md (with `dict_entity` and `ui_counter.entity_kind`), `SignalStore` (a `SignalSink`: one transaction per `write`, bounded LRU, typed errors) and the `SignalWriter` thread. Everything works except one acceptance criterion: database.md's `signal_event` schema measures 73.4 bytes/event with its indexes, not <= 60, which cannot be fixed inside the ticket scope. That test is committed `#[ignore]`d next to a regression guard, with measured alternatives below.

acceptance criteria:
- [x] Migrations apply on an empty DB and are idempotent — tests/signal_store.rs::migrations_apply_on_an_empty_database_and_are_idempotent, schema_matches_database_md, the_database_is_wal_with_incremental_auto_vacuum; migrate.rs unit tests (idempotence, rollback of a failing migration, gap/too-new/edited detection, CRLF-stable checksum); connection.rs tests (pragmas; refuses a DB created without incremental auto_vacuum).
- [x] 256-event batch write < 5 ms — `cargo bench -p tinwave-db --bench write_batch` on a DB holding 100k events; median gate, 4 runs p50 2.8 / 2.8 / 3.0 / 3.2 ms; mean 3.5–5.1 ms (not gated); p99 17–30 ms, max 100–400 ms (batch after a WAL checkpoint + Windows fsync stalls). 1 MiB cache median 2.0 ms.
- [ ] 100 000 realistic events ⇒ ≤ 60 bytes/event incl. index — measured 73.4 B/event (table 39.6, UNIQUE(device, seq) index 14.2, rec_ts index 19.6) via dbstat; whole file 78.8 B/event (7.7 MB per 100k). tests/storage_budget.rs: regression guard (75 B ceiling) passes; the 60 B test is `#[ignore]`d and fails at 73.4. Alternatives measured on the same 100k events: A baseline 73.4; ts in seconds 69.2; ts relative to session start 67.0; source only on PlayStarted 71.9; B `WITHOUT ROWID` + `PRIMARY KEY (device, seq)` 61.1; B + ts seconds 56.8; B + ts relative to session start 54.2; B + source only on PlayStarted 59.4. Synthetic mix ~9 events/play.
- [x] Writer adds < 2 MB RSS — `cargo bench -p tinwave-db --bench writer_rss`: warm SQLite code +1 336 KiB (baseline 18 688, steady/peak 20 024 KiB); cold +2.3 MB. Store sets a 256 KiB page cache (`StoreConfig::page_cache_kib`) and a 2 048-entry dictionary LRU. Sweep: 1024 KiB + 8192 LRU +2.9 MB; 512 + 4096 +2.2 MB; 256 + 2048 +1.5 MB.
- [x] Duplicate `(device, seq)` insert ignored — `a_duplicate_device_seq_is_ignored`.
- [x] Shutdown flush loses nothing accepted by the bus — `signal_writer.rs::shutdown_flushes_everything_the_bus_accepted` (1 500 events, all stored, `events_lost == 0`); `the_writer_stops_by_itself_when_every_producer_is_gone`; `shutting_down_while_the_database_fails_reports_what_was_lost`.
- [x] Ticket notes: all-or-nothing (`a_failure_mid_batch_rolls_back_events_counters_and_dictionary_rows` via a real SQLite trigger from a second connection; rolled-back dictionary ids not remembered by the LRU; retry counts once); same through a real bus (`a_failed_commit_leaves_the_store_usable_and_the_bus_retry_counts_once`); `INSERT OR IGNORE`; counter deltas via `ON CONFLICT DO UPDATE` (`counter_rows_are_deltas_added_with_the_larger_scroll_depth_kept`); track counters via dict_recording, others via dict_entity (`counters_resolve_through_the_dictionary_the_entity_kind_selects`); bounded true LRU checked against a naive model; `first_seq = max(seq)+1` per device; typed errors (`DbError` keeps `rusqlite::Error` as source; `SignalsError::Sink.reason` gets `DbError::chain()`; asserted with `downcast_ref`); flush policy 2 s / 256 events / play end (Completed, Skipped, Paused, SessionEnded) / counters-only / shutdown; failed-commit retry every flush interval with bounded memory; pragmas WAL, incremental auto_vacuum, journal_size_limit, synchronous NORMAL, foreign_keys, page_size 4096; temp dirs, ManualClock, SeededRng, IdGenerator; rusqlite `bundled`, deny passes.

checks: fmt pass, clippy pass, tests 256 passed / 0 failed / 2 ignored (one is the 60-byte test), deny pass. Benches: write_batch gate ok (median); writer_rss ok (warm).
files changed: Cargo.toml ([workspace.dependencies]: tinwave-signals, rusqlite 0.41 default-features=false + bundled + cache, tempfile, tracing 0.1 std-only), Cargo.lock; crates/tinwave-db/{Cargo.toml, migrations/signals/0001_init.sql, src/{lib,error,migrate,connection,dict,lru,store,writer}.rs, tests/{common/mod.rs, signal_store.rs, signal_writer.rs, storage_budget.rs}, benches/{write_batch,writer_rss}.rs}
signals added/changed: none. Storage measured 73.4 B/event.
ui: none
docs updated: none

deviations from ticket:
1. Staging in front of the store: `SignalDrain::wait` returns on the first event and `pump` hands over whatever is queued, so pumping the store directly would be one transaction per event. The writer pumps into an infallible in-memory `Stage` and commits the stage per the flush policy; the store's all-or-nothing contract makes retrying the kept stage correct. The bus never retains a failed batch (P-0040 backoff unused); the writer owns the retry (flush interval). `wait` timeout = min(time until due, stop_poll) ≤ flush interval.
2. `stop_poll` default 250 ms: an idle writer wakes every 250 ms to check the stop flag (no IO).
3. WAL checkpoints moved out of commits: `wal_autocheckpoint=0`; `checkpoint_if_due(idle)` runs `wal_checkpoint(TRUNCATE)` between batches (8 MiB idle, 64 MiB regardless, every 60 s if non-empty), never waits for a reader, ≥ 2 s apart. Automatic checkpoints fsync 100–800 ms on Windows. Test `the_wal_is_checkpointed_when_idle_and_large_and_never_inside_a_commit`.
4. Sessions: `SignalEvent.session` is a UUID but `session` has no uuid column; the store creates the row on first sight (`started_ts` = UUIDv7 time, `tz_offset_min` from `SignalWriter::set_tz_offset`), mapping kept in an LRU, not across restarts.
5. `Paused` counts as a play end for flushing; smaller caches for RSS; an unencodable payload is skipped, counted (`events_rejected`) and logged at warn.
6. The 60-byte test committed ignored plus a regression guard.

proposals:
- P: Decide the signal_event schema for the 60 B/event budget (WITHOUT ROWID PK(device, seq) 61.1 B; + ts relative to session ~54 B; + source only on PlayStarted 59 B) | size: S
- P: Add `uuid` to the signals.db `session` table | size: S
- P: Document the checkpoint policy and writer cache sizes | size: S
- P: `SignalDrain` pump on demand with a minimum batch | size: M
- P: Follow-up ticket for live `play` records | size: M
- P: A `tracing` subscriber in the app composition root | size: S

questions for lead:
- Reach 60 B/event by schema change or by relaxing the budget?
- OK with deviation 1 (staging)?

---
Lead answers (2026-11-02): schema change, not budget change — WITHOUT ROWID PRIMARY KEY (device, seq), `ts_off` = ms
since session.started_ts, index (recording, session, ts_off), `session.uuid` added (database.md updated). Staging,
stop_poll, checkpoint policy and Paused-as-play-end accepted (checkpoint policy documented in database.md).
