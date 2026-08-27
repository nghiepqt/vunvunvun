# Phase 01 - Data layer

Read `../CONTEXT.md` first.

## Goal

Build the complete persistence layer, including the parts that only matter once multi-device sync exists. Sync itself ships much later, but the schema decisions it needs are the ones that cannot be retrofitted. Getting this wrong means rewriting the data layer after users have data in it.

Estimated two weeks.

## Prerequisites

Phase 00, specifically the answer on sqlite-vec extension loading and the accepted primary key type.

## Why sync-shaped now

Sync is a data-model decision, not an infrastructure decision. No server, no accounts, and no sync code ship in the MVP. But six things must be right in the first migration:

1. Identifiers are client-generated UUIDv7. Never autoincrement for anything that will sync. Integer identifiers collide across devices, and changing a primary key later rewrites every foreign key and every stored reference. UUIDv7 is time-sortable so index locality survives.
2. Every syncable row carries `updated_at`, `deleted_at`, `device_id`, and `hlc`. Without `deleted_at` tombstones a delete cannot propagate and the row simply reappears on the next sync.
3. All writes to syncable tables go through an append-only op-log. With one, almost any sync strategy can be added later. Without one there is no change history to send.
4. Ordering uses a hybrid logical clock with a bounded drift check, never wall-clock time.
5. Conflict semantics are defined per field and written down now.
6. Syncable and device-local tables are separated in the first migration, because most of this application's data must never leave the machine.

## Scope

### 1. Migration infrastructure

Plain SQL migration files, applied in order, tracked in a `schema_version` table. Migration 0001 contains the entire schema below. Do not split it across files; the whole point is that the sync shape exists from the first version.

Open connections with WAL journal mode and `synchronous=NORMAL`. Use prepared statements throughout.

### 2. Syncable tables

`inbox_item` holds raw captures. Columns: `id` (UUIDv7 text), `raw_text`, `source` (hotkey, voice, image, mobile, import), `status` (pending, pending_consent, structured, discarded), `created_at`, `processed_at`, plus the four sync columns.

`raw_text` is immutable. The user's original words must always be recoverable, so the op-log rejects any `set` operation against it.

There is deliberately no audio path column here. A per-column sync exclusion has no mechanism in this design, since every write to a syncable table emits an operation. Audio paths live in the device-local `inbox_audio` table.

`task` holds tasks and their steps in one self-referencing table. Columns: `id`, `project_id`, `title`, `category`, `parent_id`, `position`, `granularity` (1 to 5, null for leaf steps), `status` (open, in_progress, done, parked), `due_at`, `due_precision` (day, hour, minute), `urgency`, `target_app_id`, `est_p50_sec`, `est_p80_sec`, `est_p90_sec`, `actual_sec`, `origin` (user, t1, t2, t3), `origin_model`, `origin_prompt_ver`, `accepted_at`, `created_at`, plus the four sync columns.

`position` is text, not a floating-point number. A float fractional index runs out of mantissa after roughly fifty-three midpoint insertions into the same gap, after which two siblings compare equal and ordering silently collapses. Use a variable-length base-62 fractional index with the device identifier appended as a tie-breaker, so two offline devices inserting after the same sibling produce different keys instead of colliding.

`due_at` is written only by the deterministic date parser, never by a model.

`accepted_at` is null until an explicit user action sets it. No code path may set it automatically.

`note` holds the non-task candidates from structuring. Columns: `id`, `kind` (note, event, question, decision), `text`, `starts_at`, `answered`, `origin`, `accepted_at`, `created_at`, plus sync columns.

`project` scopes retrieval. Columns: `id`, `name`, `archived`, `created_at`, plus sync columns.

`transcript` is opt-in syncable and holds meeting text. Columns: `id`, `text`, `started_at`, `duration_sec`, `consent_ack`, plus sync columns. Audio never syncs.

`user_profile` holds the serialized preference object. Columns: `id`, `json`, `profile_hash`, `cluster_id`, plus sync columns. `profile_hash` is a hash of only the prompt-cache-relevant subset of preferences, and `cluster_id` is the prompt-cache bucket. Both are derived and must be recomputed locally after any merge, never merged themselves; a merged hash that disagrees with the merged JSON silently poisons the prompt cache.

### 3. The op-log

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

The entity index must include `field`. Without it, resolving a single field scans every operation for the entity.

Expose exactly one write path, `OpLog.Apply()`, which writes the operation and applies it to the materialized table in one transaction. Nothing else in the codebase writes to a syncable table.

For `user_profile`, emit one operation per top-level preference key, using a dotted field name such as `json.demandStyle`, and materialize by JSON patch. Whole-blob writes lose a settings change made on another device while this one was offline.

### 4. Hybrid logical clock

Format is a lexicographically sortable string: thirteen digits of milliseconds, a dash, ten digits of counter, a dash, the device identifier. The device identifier suffix makes the ordering total, so ties break identically on every device, which is what makes last-write-wins converge.

```csharp
public sealed class Hlc
{
    private const long MaxDriftMs = 60_000;
    private const uint MaxCounter = 999_999_999u;

    private long _lastMs;
    private uint _counter;
    private readonly string _deviceId;

    public string Tick(long nowMs)
    {
        if (nowMs > _lastMs) { _lastMs = nowMs; _counter = 0; }
        else                 { _counter++; }
        if (_counter >= MaxCounter) { _lastMs++; _counter = 0; }
        return Format();
    }

    public void Observe(long remoteMs, uint remoteCounter, long nowMs)
    {
        if (remoteMs > nowMs + MaxDriftMs)
            throw new ClockDriftTooLargeException(remoteMs, nowMs);

        var maxMs = Math.Max(nowMs, Math.Max(_lastMs, remoteMs));
        _counter =
            maxMs == _lastMs && maxMs == remoteMs ? Math.Max(_counter, remoteCounter) + 1
          : maxMs == _lastMs                      ? _counter + 1
          : maxMs == remoteMs                     ? remoteCounter + 1
          :                                         0;
        _lastMs = maxMs;
    }

    private string Format() =>
        string.Create(CultureInfo.InvariantCulture,
            $"{_lastMs:D13}-{_counter:D10}-{_deviceId}");
}
```

Three requirements that are easy to miss and fatal to skip:

- The drift bound is not optional. A hybrid logical clock on its own does not prevent a device with a badly wrong clock from poisoning every peer and winning every conflict forever. The bounded check is what does.
- `_lastMs` and `_counter` must be durably persisted after every tick and restored at startup, in the `hlc_state` table. Otherwise a backwards time correction after a restart makes the clock emit values below ones this device already emitted, breaking the total order.
- The counter width in the format string must match the overflow guard. Padding widens rather than truncates, so a counter one digit wider than the format sorts below a narrower one and silently breaks ordering.

### 5. Per-field conflict rules

Per-field rules require per-field clocks. A single row-level `hlc` column cannot resolve a field-level merge, because applying an out-of-order remote operation has nothing to compare against and the result depends on arrival order.

Implement the `field_clock` table: primary key of entity type, entity id, and field name, with the winning clock value. This is preferable to deriving it from the op-log, because op-log compaction later would destroy the comparison keys.

Resolution is highest field clock wins, except:

- `task.actual_sec` takes the maximum. Time worked accumulates and the larger value is the truer one.
- `task.accepted_at` is first-write-wins. A sync must never un-accept something the user accepted.
- `task.deleted_at` beats concurrent edits. A user who deleted on one device should not see it resurrected by an edit on another.
- `task.position` compares the base-62 string, ties broken by device identifier, so two offline inserts after the same sibling both survive in a deterministic order.
- `inbox_item.raw_text` is immutable; reject the operation.
- `note.answered` is true-wins, because answering is monotonic.

### 6. Device-local tables

These carry no sync columns and are written normally, not through the op-log.

`inbox_audio` maps an inbox item to a local audio path. `hlc_state` persists the clock. `field_clock` as above. `device` records this machine's identifier and name, generated once at first run.

`fragment` is the unit of retrieval: id, entity type, entity id, text, created_at. It is derived and rebuilt from syncable rows, so it never syncs; shipping vectors over the wire would be absurd.

`fragment_fts` is an FTS5 external-content table over `fragment`. Use `tokenize='unicode61 remove_diacritics 0'`. Stripping diacritics conflates distinct Vietnamese words that differ only by tone mark, which destroys the keyword half of hybrid retrieval. External-content FTS5 requires explicit insert, update, and delete triggers or the index silently drifts out of sync; write all three.

`fragment_vec` is the sqlite-vec virtual table, dimension 384 to match the embedding model chosen in the AI phase. Treat the dimension as a migration-versioned constant, not a literal to be casually changed.

`activity_sample`, `energy_report`, `nudge_event`, `intervention`, `llm_call`, `response_cache`, and `ml_state` are all device-local. Their columns are defined in the phases that introduce them; create them now as part of migration 0001 so the local-versus-syncable split is complete from the start.

`energy_report` carries a check constraint that its source column equals `self_report`. This is deliberate and must not be relaxed.

`activity_sample` deliberately has no window title column. Titles add nothing to the stall detector's feature set and increase the blast radius of a breach for no model benefit.

### 7. Repositories

One repository class per aggregate, all async, all going through `OpLog.Apply()` for syncable writes. Keep query logic in the repository; do not leak SQL into services.

## Out of scope

No sync transport, no server, no accounts. No retrieval logic beyond creating the tables. No embedding generation.

## Definition of done

- Migration 0001 applies cleanly and creates every table above.
- `OpLog.Apply()` is the only write path to syncable tables, enforced by a test that greps the compiled sources or by an architecture test.
- The clock round-trips through persistence and rejects out-of-drift remote values.
- Convergence property tests pass, in two parts, because exhaustive permutation is factorial and ten operations already means over three million orderings:
  - Exhaustive permutations for six or fewer operations over a hand-built adversarial set: concurrent edits to different fields of one row, delete versus edit, two devices inserting after the same sibling, and races on `accepted_at`.
  - Randomized shuffles with shrinking for larger sets.
  - Both assert identical final state on two simulated devices.
- Tombstone propagation, clock skew beyond the drift bound being rejected rather than absorbed, and persistence across a simulated restart with a backwards time correction all have tests.
- FTS5 triggers keep the index consistent under insert, update, and delete, with a test.
