-- Migration 0001: Initial sync-shaped schema, FTS5/sqlite-vec fragment store, and device-local tables.

-- ============================================================================
-- Device-local sync metadata & append-only op-log
-- ============================================================================

CREATE TABLE IF NOT EXISTS device (
    id         TEXT PRIMARY KEY,
    name       TEXT,
    created_at INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS hlc_state (
    id        INTEGER PRIMARY KEY CHECK (id = 1),
    device_id TEXT NOT NULL,
    last_ms   INTEGER NOT NULL,
    counter   INTEGER NOT NULL CHECK (counter >= 0 AND counter <= 999999999)
);

CREATE TABLE IF NOT EXISTS field_clock (
    entity_type TEXT NOT NULL,
    entity_id   TEXT NOT NULL,
    field       TEXT NOT NULL,
    hlc         TEXT NOT NULL,
    PRIMARY KEY (entity_type, entity_id, field)
);

CREATE TABLE IF NOT EXISTS op_log (
    seq         INTEGER PRIMARY KEY AUTOINCREMENT,
    op_id       TEXT NOT NULL UNIQUE,
    hlc         TEXT NOT NULL,
    device_id   TEXT NOT NULL,
    entity_type TEXT NOT NULL CHECK (entity_type IN ('task', 'note', 'inbox_item', 'user_profile', 'transcript', 'project')),
    entity_id   TEXT NOT NULL,
    op          TEXT NOT NULL CHECK (op IN ('create', 'set', 'delete')),
    field       TEXT,
    value       TEXT,
    applied     INTEGER NOT NULL DEFAULT 1 CHECK (applied IN (0, 1)),
    synced_at   INTEGER
);

CREATE INDEX IF NOT EXISTS idx_oplog_unsynced ON op_log(seq) WHERE synced_at IS NULL;
CREATE INDEX IF NOT EXISTS idx_oplog_entity   ON op_log(entity_type, entity_id, field, hlc);

-- ============================================================================
-- Syncable tables (writes permitted ONLY through ef_oplog::OpLog)
-- ============================================================================

CREATE TABLE IF NOT EXISTS inbox_item (
    id           TEXT PRIMARY KEY,
    raw_text     TEXT NOT NULL,
    source       TEXT NOT NULL CHECK (source IN ('hotkey', 'voice', 'image', 'mobile', 'import')),
    status       TEXT NOT NULL CHECK (status IN ('pending', 'pending_consent', 'structured', 'discarded')),
    created_at   INTEGER NOT NULL,
    processed_at INTEGER,
    updated_at   INTEGER NOT NULL,
    deleted_at   INTEGER,
    device_id    TEXT NOT NULL,
    hlc          TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_inbox_item_status_created
    ON inbox_item(status, created_at DESC)
    WHERE deleted_at IS NULL;

CREATE TABLE IF NOT EXISTS project (
    id         TEXT PRIMARY KEY,
    name       TEXT NOT NULL,
    archived   INTEGER NOT NULL DEFAULT 0 CHECK (archived IN (0, 1)),
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    deleted_at INTEGER,
    device_id  TEXT NOT NULL,
    hlc        TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS task (
    id                 TEXT PRIMARY KEY,
    project_id         TEXT,
    title              TEXT NOT NULL,
    category           TEXT NOT NULL DEFAULT 'general' CHECK (category IN ('admin', 'deep_work', 'communication', 'errand', 'creative', 'general')),
    parent_id          TEXT,
    position           TEXT NOT NULL,
    position_device_id TEXT NOT NULL,
    granularity        INTEGER CHECK (granularity IS NULL OR (granularity BETWEEN 1 AND 5)),
    status             TEXT NOT NULL CHECK (status IN ('open', 'in_progress', 'done', 'parked')),
    due_at             INTEGER,
    due_precision      TEXT CHECK (due_precision IS NULL OR due_precision IN ('day', 'hour', 'minute')),
    urgency            TEXT,
    target_app_id      TEXT,
    est_p50_sec        INTEGER CHECK (est_p50_sec IS NULL OR est_p50_sec >= 0),
    est_p80_sec        INTEGER CHECK (est_p80_sec IS NULL OR est_p80_sec >= 0),
    est_p90_sec        INTEGER CHECK (est_p90_sec IS NULL OR est_p90_sec >= 0),
    actual_sec         INTEGER NOT NULL DEFAULT 0 CHECK (actual_sec >= 0),
    origin             TEXT NOT NULL CHECK (origin IN ('user', 't1', 't2', 't3')),
    origin_model       TEXT,
    origin_prompt_ver  TEXT,
    accepted_at        INTEGER,
    created_at         INTEGER NOT NULL,
    updated_at         INTEGER NOT NULL,
    deleted_at         INTEGER,
    device_id          TEXT NOT NULL,
    hlc                TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_task_parent_pos
    ON task(parent_id, position, position_device_id, id)
    WHERE deleted_at IS NULL;

CREATE INDEX IF NOT EXISTS idx_task_status
    ON task(status)
    WHERE deleted_at IS NULL;

CREATE TABLE IF NOT EXISTS note (
    id                TEXT PRIMARY KEY,
    kind              TEXT NOT NULL CHECK (kind IN ('note', 'event', 'question', 'decision')),
    text              TEXT NOT NULL,
    starts_at         INTEGER,
    answered          INTEGER NOT NULL DEFAULT 0 CHECK (answered IN (0, 1)),
    origin            TEXT NOT NULL CHECK (origin IN ('user', 't1', 't2', 't3')),
    origin_model      TEXT,
    origin_prompt_ver TEXT,
    accepted_at       INTEGER,
    created_at        INTEGER NOT NULL,
    updated_at        INTEGER NOT NULL,
    deleted_at        INTEGER,
    device_id         TEXT NOT NULL,
    hlc               TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS transcript (
    id           TEXT PRIMARY KEY,
    text         TEXT NOT NULL,
    started_at   INTEGER NOT NULL,
    duration_sec INTEGER NOT NULL CHECK (duration_sec >= 0),
    consent_ack  INTEGER NOT NULL CHECK (consent_ack IN (0, 1)),
    created_at   INTEGER NOT NULL,
    updated_at   INTEGER NOT NULL,
    deleted_at   INTEGER,
    device_id    TEXT NOT NULL,
    hlc          TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS user_profile (
    id           TEXT PRIMARY KEY,
    json         TEXT NOT NULL,
    profile_hash TEXT NOT NULL,
    cluster_id   TEXT NOT NULL,
    updated_at   INTEGER NOT NULL,
    deleted_at   INTEGER,
    device_id    TEXT NOT NULL,
    hlc          TEXT NOT NULL
);

-- ============================================================================
-- Device-local tables (never synced)
-- ============================================================================

CREATE TABLE IF NOT EXISTS inbox_audio (
    id            TEXT PRIMARY KEY,
    inbox_item_id TEXT NOT NULL,
    file_path     TEXT NOT NULL,
    duration_ms   INTEGER CHECK (duration_ms IS NULL OR duration_ms >= 0),
    retained      INTEGER NOT NULL DEFAULT 0 CHECK (retained IN (0, 1)),
    created_at    INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS fragment (
    local_rowid        INTEGER PRIMARY KEY AUTOINCREMENT,
    id                 TEXT NOT NULL UNIQUE,
    source_entity_type TEXT NOT NULL,
    source_entity_id   TEXT NOT NULL,
    text               TEXT NOT NULL,
    created_at         INTEGER NOT NULL,
    updated_at         INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_fragment_source
    ON fragment(source_entity_type, source_entity_id);

CREATE VIRTUAL TABLE IF NOT EXISTS fragment_fts USING fts5(
    text,
    content='fragment',
    content_rowid='local_rowid',
    tokenize='unicode61 remove_diacritics 0'
);

CREATE TRIGGER IF NOT EXISTS trg_fragment_fts_insert AFTER INSERT ON fragment BEGIN
    INSERT INTO fragment_fts(rowid, text) VALUES (new.local_rowid, new.text);
END;

CREATE TRIGGER IF NOT EXISTS trg_fragment_fts_delete AFTER DELETE ON fragment BEGIN
    INSERT INTO fragment_fts(fragment_fts, rowid, text) VALUES ('delete', old.local_rowid, old.text);
END;

CREATE TRIGGER IF NOT EXISTS trg_fragment_fts_update AFTER UPDATE ON fragment BEGIN
    INSERT INTO fragment_fts(fragment_fts, rowid, text) VALUES ('delete', old.local_rowid, old.text);
    INSERT INTO fragment_fts(rowid, text) VALUES (new.local_rowid, new.text);
END;

CREATE VIRTUAL TABLE IF NOT EXISTS fragment_vec USING vec0(
    embedding float[384] distance_metric=cosine
);

-- Privacy invariant: activity_sample intentionally has NO window_title column.
CREATE TABLE IF NOT EXISTS activity_sample (
    id             INTEGER PRIMARY KEY AUTOINCREMENT,
    sampled_at     INTEGER NOT NULL,
    process_name   TEXT NOT NULL,
    idle_sec       INTEGER NOT NULL CHECK (idle_sec >= 0),
    session_locked INTEGER NOT NULL DEFAULT 0 CHECK (session_locked IN (0, 1))
);

CREATE INDEX IF NOT EXISTS idx_activity_sample_time ON activity_sample(sampled_at DESC);

CREATE TABLE IF NOT EXISTS energy_report (
    id          TEXT PRIMARY KEY,
    recorded_at INTEGER NOT NULL,
    level       INTEGER NOT NULL CHECK (level BETWEEN 1 AND 5),
    scale       TEXT NOT NULL CHECK (scale IN ('two', 'three', 'five')),
    source      TEXT NOT NULL CHECK (source = 'self_report'),
    note        TEXT
);

CREATE TABLE IF NOT EXISTS nudge_event (
    id           TEXT PRIMARY KEY,
    triggered_at INTEGER NOT NULL,
    kind         TEXT NOT NULL,
    task_id      TEXT,
    outcome      TEXT,
    responded_at INTEGER
);

CREATE TABLE IF NOT EXISTS intervention (
    id           TEXT PRIMARY KEY,
    triggered_at INTEGER NOT NULL,
    kind         TEXT NOT NULL,
    task_id      TEXT,
    accepted     INTEGER CHECK (accepted IS NULL OR accepted IN (0, 1)),
    notes        TEXT
);

CREATE TABLE IF NOT EXISTS llm_call (
    id                 TEXT PRIMARY KEY,
    called_at          INTEGER NOT NULL,
    feature            TEXT NOT NULL,
    tier               TEXT NOT NULL CHECK (tier IN ('t0', 't1', 't2', 't3')),
    model              TEXT NOT NULL,
    prompt_ver         TEXT NOT NULL,
    input_tokens       INTEGER NOT NULL CHECK (input_tokens >= 0),
    cached_tokens      INTEGER NOT NULL DEFAULT 0 CHECK (cached_tokens >= 0),
    output_tokens      INTEGER NOT NULL CHECK (output_tokens >= 0),
    latency_ms         INTEGER NOT NULL CHECK (latency_ms >= 0),
    estimated_cost_usd REAL NOT NULL CHECK (estimated_cost_usd >= 0.0),
    stop_reason        TEXT NOT NULL CHECK (stop_reason IN ('stop', 'max_tokens', 'refusal', 'other')),
    truncated          INTEGER NOT NULL CHECK (truncated IN (0, 1)),
    valid              INTEGER NOT NULL CHECK (valid IN (0, 1)),
    retried            INTEGER NOT NULL CHECK (retried IN (0, 1))
);

CREATE INDEX IF NOT EXISTS idx_llm_call_called_at ON llm_call(called_at DESC);

CREATE TABLE IF NOT EXISTS response_cache (
    cache_key             TEXT PRIMARY KEY,
    feature               TEXT NOT NULL,
    prompt_ver            TEXT NOT NULL,
    profile_hash          TEXT NOT NULL,
    normalized_input_hash TEXT NOT NULL,
    granularity           INTEGER,
    response_json         TEXT NOT NULL,
    created_at            INTEGER NOT NULL,
    expires_at            INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_response_cache_expires ON response_cache(expires_at);

CREATE TABLE IF NOT EXISTS ml_state (
    model_key  TEXT PRIMARY KEY,
    state_json TEXT NOT NULL,
    updated_at INTEGER NOT NULL
);
