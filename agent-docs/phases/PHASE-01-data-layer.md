# Phase 01 - Data layer

Read `../CONTEXT.md` first.

## Goal

Build the complete persistence layer in `ef-core`, `ef-oplog`, and `ef-storage`, including the parts that only matter once multi-device sync exists. Sync itself ships much later, but the schema decisions it needs cannot be retrofitted without rewriting the data layer after users have data in it.

Estimated two weeks.

## Prerequisites

Phase 00, specifically Spike W4 (`sqlite-vec` static linking + `vec0` key strategy and SQLCipher verification).

## Why sync-shaped now

Sync is a data-model decision, not an infrastructure decision. No server, no accounts, and no sync code ship in the MVP. But six things must be right in the first migration:

1. Identifiers are client-generated UUIDv7 (`uuid::Uuid::now_v7()`). Never autoincrement for anything that will sync. Integer identifiers collide across devices, and UUIDv7 preserves B-tree time-locality.
2. Every syncable row carries `updated_at`, `deleted_at` (tombstone), `device_id`, and `hlc`. Without tombstones a delete cannot propagate and resurrects on the next sync.
3. All writes to syncable tables go through an append-only op-log (`ef_oplog::OpLog::apply`).
4. Ordering uses a Hybrid Logical Clock with a bounded drift check, never wall-clock time.
5. Conflict semantics are defined per field and stored with per-field clocks.
6. Syncable and device-local tables are separated in the first migration, because most of this application's data must never leave the machine.

## Scope

### 1. Migration infrastructure & connection model

Plain SQL migration files embedded via `include_str!` in `ef-storage`, applied in order, tracked in `schema_version`. Migration `0001_init.sql` creates the entire initial schema.

- Encrypt the SQLite database at rest using SQLCipher (`bundled-sqlcipher-vendored-openssl`, GDPR Art. 9) with the random 256-bit key stored in Windows Credential Manager (`CredWriteW`/`CredReadW` inside `ef-win32`).
- Open connections with `PRAGMA journal_mode = WAL;` and `PRAGMA synchronous = NORMAL;`.
- Serialize writes through a dedicated SQLite writer `std::thread` (never holding a `rusqlite::Connection` mutex across a `tokio` `.await`), with a small read-only connection pool for concurrent queries.

### 2. Syncable tables

- `inbox_item`: `id` (UUIDv7 text), `raw_text`, `source` (`hotkey`, `voice`, `image`, `mobile`, `import`), `status` (`pending`, `pending_consent`, `structured`, `discarded`), `created_at`, `processed_at`, plus sync columns.
  - `raw_text` is immutable: `ef_oplog::OpLog::apply` rejects any `SetField` targeting `raw_text`.
  - Audio paths never live here; they live in device-local `inbox_audio`.
- `task`: self-referencing tasks and steps. Columns: `id`, `project_id`, `title`, `category`, `parent_id`, `position` (variable-length base-62 text fractional index + `device_id` tie-breaker), `granularity` (1–5, null for leaf steps), `status` (`open`, `in_progress`, `done`, `parked`), `due_at`, `due_precision` (`day`, `hour`, `minute`), `urgency`, `target_app_id`, `est_p50_sec`, `est_p80_sec`, `est_p90_sec`, `actual_sec`, `origin` (`user`, `t1`, `t2`, `t3`), `origin_model`, `origin_prompt_ver`, `accepted_at`, `created_at`, plus sync columns.
  - `due_at` is written only by the deterministic date parser, never by a model.
  - `accepted_at` requires `ef_core::UserAcceptedProof` in `SyncMutation` and is never auto-set.
- `note`: `id`, `kind` (`note`, `event`, `question`, `decision`), `text`, `starts_at`, `answered`, `origin`, `accepted_at`, `created_at`, plus sync columns.
- `project`: `id`, `name`, `archived`, `created_at`, plus sync columns.
- `transcript`: opt-in syncable meeting text (`id`, `text`, `started_at`, `duration_sec`, `consent_ack`, plus sync columns). Audio never syncs.
- `user_profile`: `id`, `json`, `profile_hash`, `cluster_id`, plus sync columns. Emit one op per top-level preference key (`json.demandStyle`) and materialize via JSON merge/patch; recompute `profile_hash` and `cluster_id` locally after every merge.

### 3. The op-log and compile-time isolation

```sql
CREATE TABLE op_log (
  seq         INTEGER PRIMARY KEY AUTOINCREMENT,  -- local ordering only, never synced
  op_id       TEXT NOT NULL UNIQUE,               -- UUIDv7, globally unique
  hlc         TEXT NOT NULL,                      -- total order across devices
  device_id   TEXT NOT NULL,
  entity_type TEXT NOT NULL,   -- task|note|inbox_item|user_profile|transcript|project
  entity_id   TEXT NOT NULL,
  op          TEXT NOT NULL,   -- create|set|delete
  field       TEXT,            -- null for create and delete, field name for set
  value       TEXT,            -- JSON encoded
  applied     INTEGER NOT NULL DEFAULT 1,
  synced_at   INTEGER          -- null until the server acknowledges, phase 5
);
CREATE INDEX idx_oplog_unsynced ON op_log(seq) WHERE synced_at IS NULL;
CREATE INDEX idx_oplog_entity   ON op_log(entity_type, entity_id, field, hlc);
```

`ef-oplog` owns a wrapper `SyncWriteConn` whose inner `rusqlite::Connection` field is private to `ef-oplog`. Outside crates can read via read-only handles and mutate syncable tables **exclusively** by passing `SyncMutation` to `OpLog::apply()`, which writes `op_log`, updates `field_clock`, and materializes the target row inside one SQLite transaction.

### 4. Hybrid logical clock (`ef_core::hlc::Hlc`)

Lexicographically sortable string: `{last_ms:013}-{counter:010}-{device_id}`.

```rust
pub struct Hlc {
    last_ms: i64,
    counter: u32,
    device_id: String,
}

impl Hlc {
    pub const MAX_DRIFT_MS: i64 = 60_000;
    pub const MAX_COUNTER: u32 = 999_999_999;

    pub fn tick(&mut self, now_ms: i64) -> String {
        if now_ms > self.last_ms {
            self.last_ms = now_ms;
            self.counter = 0;
        } else {
            self.counter += 1;
        }
        if self.counter >= Self::MAX_COUNTER {
            self.last_ms += 1;
            self.counter = 0;
        }
        self.format()
    }

    pub fn observe(
        &mut self,
        remote_ms: i64,
        remote_counter: u32,
        now_ms: i64,
    ) -> Result<(), ClockDriftTooLargeError> {
        if remote_ms > now_ms + Self::MAX_DRIFT_MS {
            return Err(ClockDriftTooLargeError { remote_ms, now_ms });
        }
        let max_ms = now_ms.max(self.last_ms).max(remote_ms);
        self.counter = if max_ms == self.last_ms && max_ms == remote_ms {
            self.counter.max(remote_counter) + 1
        } else if max_ms == self.last_ms {
            self.counter + 1
        } else if max_ms == remote_ms {
            remote_counter + 1
        } else {
            0
        };
        self.last_ms = max_ms;
        Ok(())
    }

    fn format(&self) -> String {
        format!("{:013}-{:010}-{}", self.last_ms, self.counter, self.device_id)
    }
}
```

Three mandatory invariants:
- `MAX_DRIFT_MS = 60_000` is mandatory so a rogue fast clock cannot poison peers.
- `(last_ms, counter)` must be durably persisted in `hlc_state` after every tick inside the same transaction as the op-log write and restored at startup.
- Width `{:010}` must match `MAX_COUNTER = 999_999_999` so padding never widens past 10 digits.

### 5. Per-field conflict rules

Store winning clocks in `field_clock(entity_type, entity_id, field, hlc)`. Resolution is highest `hlc` wins, except:
- `task.actual_sec`: `max`.
- `task.accepted_at`: first-write-wins (a sync must never un-accept user-accepted work).
- `task.deleted_at`: delete beats concurrent edits.
- `task.position`: base-62 string comparison with `device_id` tie-breaker.
- `inbox_item.raw_text`: immutable; reject `SetField`.
- `note.answered`: `true` wins.

### 6. Device-local tables (`ef-storage`)

Written normally (never via `op_log`):
- `inbox_audio`, `hlc_state`, `field_clock`, `device`.
- `fragment` + `fragment_fts` (`FTS5`, `tokenize='unicode61 remove_diacritics 0'` to preserve Vietnamese diacritics, plus explicit insert/update/delete triggers) + `fragment_vec` (`vec0`, 384 dims for `multilingual-e5-small`).
- `activity_sample` (no window title column), `energy_report` (`CHECK (source = 'self_report')`), `nudge_event`, `intervention`, `llm_call`, `response_cache`, `ml_state`.

### 7. Repositories

One repository struct per aggregate in `ef-oplog` / `ef-storage`, exposing async methods that dispatch onto the dedicated SQLite writer or read pool. No raw SQL in `ef-app` or `ef-ai`.

## Out of scope

No sync transport, no server, no accounts. No retrieval scoring beyond table/trigger creation. No embedding generation.

## Definition of done

- Migration `0001_init.sql` applies cleanly on SQLCipher and creates all tables, indexes, check constraints, and FTS5 triggers.
- `OpLog::apply()` is the sole write path to syncable tables, enforced by Rust visibility (`SyncWriteConn`) and CI policy check.
- `Hlc` round-trips through `hlc_state`, survives simulated backwards NTP jump on restart, and rejects drift beyond `60_000` ms.
- `proptest` convergence suite passes:
  - Exhaustive permutations for `n <= 6` over adversarial ops (concurrent field edits, delete vs edit, sibling insertion ties, `accepted_at` races).
  - Randomized shuffles with shrinking for larger op sets.
  - Both assert identical final state across two simulated devices.
- FTS5 triggers maintain index consistency across insert, update, and delete with Vietnamese diacritics intact.
