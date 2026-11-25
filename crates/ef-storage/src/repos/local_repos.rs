use ef_core::EnergyScale;
use ef_protocol::{AiTier, FeatureId, LlmUsageRecord, StopReason};
use rusqlite::{OptionalExtension as _, Row, params};
use uuid::Uuid;

use crate::db::{Database, current_unix_ms};
use crate::error::{EMBEDDING_DIM, StorageError};

fn parse_uuid(raw: &str) -> Result<Uuid, StorageError> {
    Uuid::parse_str(raw).map_err(|e| StorageError::InvalidUuid(format!("{raw}: {e}")))
}

fn embedding_to_le_bytes(embedding: &[f32]) -> Result<Vec<u8>, StorageError> {
    if embedding.len() != EMBEDDING_DIM {
        return Err(StorageError::InvalidEmbeddingDim {
            expected: EMBEDDING_DIM,
            actual: embedding.len(),
        });
    }
    let mut bytes = Vec::with_capacity(EMBEDDING_DIM * 4);
    for &val in embedding {
        bytes.extend_from_slice(&val.to_le_bytes());
    }
    Ok(bytes)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fragment {
    pub local_rowid: i64,
    pub id: Uuid,
    pub source_entity_type: String,
    pub source_entity_id: String,
    pub text: String,
    pub created_at: i64,
    pub updated_at: i64,
}

fn map_fragment_row(row: &Row<'_>) -> Result<Fragment, StorageError> {
    let id_str: String = row.get(1)?;
    Ok(Fragment {
        local_rowid: row.get(0)?,
        id: parse_uuid(&id_str)?,
        source_entity_type: row.get(2)?,
        source_entity_id: row.get(3)?,
        text: row.get(4)?,
        created_at: row.get(5)?,
        updated_at: row.get(6)?,
    })
}

#[derive(Clone)]
pub struct FragmentRepository {
    db: Database,
}

impl FragmentRepository {
    #[must_use]
    pub fn new(db: Database) -> Self {
        Self { db }
    }

    pub async fn insert(
        &self,
        id: Uuid,
        source_entity_type: String,
        source_entity_id: String,
        text: String,
        embedding: Option<&[f32]>,
    ) -> Result<i64, StorageError> {
        let emb_bytes = embedding.map(embedding_to_le_bytes).transpose()?;
        let now_ms = current_unix_ms();
        let id_str = id.to_string();
        self.db
            .write_local(move |conn| {
                let tx = conn.transaction()?;
                tx.execute(
                    "INSERT INTO fragment(id, source_entity_type, source_entity_id, text, created_at, updated_at)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?5)",
                    params![id_str, source_entity_type, source_entity_id, text, now_ms],
                )?;
                let local_rowid = tx.last_insert_rowid();
                if let Some(bytes) = emb_bytes {
                    tx.execute(
                        "INSERT INTO fragment_vec(rowid, embedding) VALUES (?1, ?2)",
                        params![local_rowid, bytes],
                    )?;
                }
                tx.commit()?;
                Ok(local_rowid)
            })
            .await
    }

    pub async fn update_text(&self, id: Uuid, text: String) -> Result<(), StorageError> {
        let now_ms = current_unix_ms();
        let id_str = id.to_string();
        self.db
            .write_local(move |conn| {
                let updated = conn.execute(
                    "UPDATE fragment SET text = ?2, updated_at = ?3 WHERE id = ?1",
                    params![id_str, text, now_ms],
                )?;
                if updated == 0 {
                    return Err(StorageError::NotFound(format!("fragment {id_str}")));
                }
                Ok(())
            })
            .await
    }

    pub async fn upsert_embedding(&self, id: Uuid, embedding: &[f32]) -> Result<(), StorageError> {
        let bytes = embedding_to_le_bytes(embedding)?;
        let id_str = id.to_string();
        self.db
            .write_local(move |conn| {
                let tx = conn.transaction()?;
                let local_rowid: Option<i64> = tx
                    .query_row(
                        "SELECT local_rowid FROM fragment WHERE id = ?1",
                        params![id_str],
                        |row| row.get(0),
                    )
                    .optional()?;
                let Some(rowid) = local_rowid else {
                    return Err(StorageError::NotFound(format!("fragment {id_str}")));
                };
                tx.execute("DELETE FROM fragment_vec WHERE rowid = ?1", params![rowid])?;
                tx.execute(
                    "INSERT INTO fragment_vec(rowid, embedding) VALUES (?1, ?2)",
                    params![rowid, bytes],
                )?;
                tx.commit()?;
                Ok(())
            })
            .await
    }

    pub async fn delete(&self, id: Uuid) -> Result<(), StorageError> {
        let id_str = id.to_string();
        self.db
            .write_local(move |conn| {
                let tx = conn.transaction()?;
                let local_rowid: Option<i64> = tx
                    .query_row(
                        "SELECT local_rowid FROM fragment WHERE id = ?1",
                        params![id_str],
                        |row| row.get(0),
                    )
                    .optional()?;
                if let Some(rowid) = local_rowid {
                    tx.execute("DELETE FROM fragment_vec WHERE rowid = ?1", params![rowid])?;
                    tx.execute(
                        "DELETE FROM fragment WHERE local_rowid = ?1",
                        params![rowid],
                    )?;
                }
                tx.commit()?;
                Ok(())
            })
            .await
    }

    pub async fn search_fts(
        &self,
        query: String,
        limit: usize,
    ) -> Result<Vec<Fragment>, StorageError> {
        let limit_i64 = i64::try_from(limit).unwrap_or(i64::MAX);
        self.db
            .read(move |conn| {
                let mut stmt = conn.prepare(
                    "SELECT f.local_rowid, f.id, f.source_entity_type, f.source_entity_id, f.text, f.created_at, f.updated_at
                     FROM fragment_fts
                     JOIN fragment f ON f.local_rowid = fragment_fts.rowid
                     WHERE fragment_fts MATCH ?1
                     ORDER BY bm25(fragment_fts) ASC
                     LIMIT ?2",
                )?;
                let mut rows = stmt.query(params![query, limit_i64])?;
                let mut out = Vec::new();
                while let Some(row) = rows.next()? {
                    out.push(map_fragment_row(row)?);
                }
                Ok(out)
            })
            .await
    }

    pub async fn search_knn(
        &self,
        query_embedding: &[f32],
        limit: usize,
    ) -> Result<Vec<(Fragment, f64)>, StorageError> {
        let bytes = embedding_to_le_bytes(query_embedding)?;
        let limit_i64 = i64::try_from(limit).unwrap_or(i64::MAX);
        self.db
            .read(move |conn| {
                let mut stmt = conn.prepare(
                    "SELECT f.local_rowid, f.id, f.source_entity_type, f.source_entity_id, f.text, f.created_at, f.updated_at, v.distance
                     FROM fragment_vec v
                     JOIN fragment f ON f.local_rowid = v.rowid
                     WHERE v.embedding MATCH ?1 AND k = ?2
                     ORDER BY v.distance ASC",
                )?;
                let mut rows = stmt.query(params![bytes, limit_i64])?;
                let mut out = Vec::new();
                while let Some(row) = rows.next()? {
                    let frag = map_fragment_row(row)?;
                    let dist: f64 = row.get(7)?;
                    out.push((frag, dist));
                }
                Ok(out)
            })
            .await
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InboxAudioRecord {
    pub id: Uuid,
    pub inbox_item_id: Uuid,
    pub file_path: String,
    pub duration_ms: Option<i64>,
    pub retained: bool,
    pub created_at: i64,
}

#[derive(Clone)]
pub struct InboxAudioRepository {
    db: Database,
}

impl InboxAudioRepository {
    #[must_use]
    pub fn new(db: Database) -> Self {
        Self { db }
    }

    pub async fn insert(
        &self,
        id: Uuid,
        inbox_item_id: Uuid,
        file_path: String,
        duration_ms: Option<i64>,
        retained: bool,
    ) -> Result<(), StorageError> {
        let now_ms = current_unix_ms();
        self.db
            .write_local(move |conn| {
                conn.execute(
                    "INSERT INTO inbox_audio(id, inbox_item_id, file_path, duration_ms, retained, created_at)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                    params![
                        id.to_string(),
                        inbox_item_id.to_string(),
                        file_path,
                        duration_ms,
                        i64::from(retained),
                        now_ms
                    ],
                )?;
                Ok(())
            })
            .await
    }

    pub async fn list_for_inbox_item(
        &self,
        inbox_item_id: Uuid,
    ) -> Result<Vec<InboxAudioRecord>, StorageError> {
        let item_str = inbox_item_id.to_string();
        self.db
            .read(move |conn| {
                let mut stmt = conn.prepare(
                    "SELECT id, inbox_item_id, file_path, duration_ms, retained, created_at
                     FROM inbox_audio WHERE inbox_item_id = ?1 ORDER BY created_at ASC",
                )?;
                let mut rows = stmt.query(params![item_str])?;
                let mut out = Vec::new();
                while let Some(row) = rows.next()? {
                    let id_str: String = row.get(0)?;
                    let item_id_str: String = row.get(1)?;
                    let retained_i64: i64 = row.get(4)?;
                    out.push(InboxAudioRecord {
                        id: parse_uuid(&id_str)?,
                        inbox_item_id: parse_uuid(&item_id_str)?,
                        file_path: row.get(2)?,
                        duration_ms: row.get(3)?,
                        retained: retained_i64 != 0,
                        created_at: row.get(5)?,
                    });
                }
                Ok(out)
            })
            .await
    }

    pub async fn delete(&self, id: Uuid) -> Result<(), StorageError> {
        let id_str = id.to_string();
        self.db
            .write_local(move |conn| {
                conn.execute("DELETE FROM inbox_audio WHERE id = ?1", params![id_str])?;
                Ok(())
            })
            .await
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActivitySample {
    pub id: i64,
    pub sampled_at: i64,
    pub process_name: String,
    pub idle_sec: i64,
    pub session_locked: bool,
}

#[derive(Clone)]
pub struct ActivitySampleRepository {
    db: Database,
}

impl ActivitySampleRepository {
    #[must_use]
    pub fn new(db: Database) -> Self {
        Self { db }
    }

    pub async fn record(
        &self,
        sampled_at: i64,
        process_name: String,
        idle_sec: i64,
        session_locked: bool,
    ) -> Result<i64, StorageError> {
        self.db
            .write_local(move |conn| {
                conn.execute(
                    "INSERT INTO activity_sample(sampled_at, process_name, idle_sec, session_locked)
                     VALUES (?1, ?2, ?3, ?4)",
                    params![sampled_at, process_name, idle_sec, i64::from(session_locked)],
                )?;
                Ok(conn.last_insert_rowid())
            })
            .await
    }

    pub async fn list_recent(&self, limit: usize) -> Result<Vec<ActivitySample>, StorageError> {
        let limit_i64 = i64::try_from(limit).unwrap_or(i64::MAX);
        self.db
            .read(move |conn| {
                let mut stmt = conn.prepare(
                    "SELECT id, sampled_at, process_name, idle_sec, session_locked
                     FROM activity_sample ORDER BY sampled_at DESC LIMIT ?1",
                )?;
                let mut rows = stmt.query(params![limit_i64])?;
                let mut out = Vec::new();
                while let Some(row) = rows.next()? {
                    let locked_i64: i64 = row.get(4)?;
                    out.push(ActivitySample {
                        id: row.get(0)?,
                        sampled_at: row.get(1)?,
                        process_name: row.get(2)?,
                        idle_sec: row.get(3)?,
                        session_locked: locked_i64 != 0,
                    });
                }
                Ok(out)
            })
            .await
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EnergyReport {
    pub id: Uuid,
    pub recorded_at: i64,
    pub level: u8,
    pub scale: EnergyScale,
    pub note: Option<String>,
}

#[derive(Clone)]
pub struct EnergyReportRepository {
    db: Database,
}

impl EnergyReportRepository {
    #[must_use]
    pub fn new(db: Database) -> Self {
        Self { db }
    }

    pub async fn record_self_report(
        &self,
        id: Uuid,
        recorded_at: i64,
        level: u8,
        scale: EnergyScale,
        note: Option<String>,
    ) -> Result<(), StorageError> {
        let scale_str = match scale {
            EnergyScale::Two => "two",
            EnergyScale::Three => "three",
            EnergyScale::Five => "five",
        };
        self.db
            .write_local(move |conn| {
                conn.execute(
                    "INSERT INTO energy_report(id, recorded_at, level, scale, source, note)
                     VALUES (?1, ?2, ?3, ?4, 'self_report', ?5)",
                    params![
                        id.to_string(),
                        recorded_at,
                        i64::from(level),
                        scale_str,
                        note
                    ],
                )?;
                Ok(())
            })
            .await
    }

    pub async fn list_recent(&self, limit: usize) -> Result<Vec<EnergyReport>, StorageError> {
        let limit_i64 = i64::try_from(limit).unwrap_or(i64::MAX);
        self.db
            .read(move |conn| {
                let mut stmt = conn.prepare(
                    "SELECT id, recorded_at, level, scale, note
                     FROM energy_report ORDER BY recorded_at DESC LIMIT ?1",
                )?;
                let mut rows = stmt.query(params![limit_i64])?;
                let mut out = Vec::new();
                while let Some(row) = rows.next()? {
                    let id_str: String = row.get(0)?;
                    let level_i64: i64 = row.get(2)?;
                    let scale_str: String = row.get(3)?;
                    let scale = match scale_str.as_str() {
                        "two" => EnergyScale::Two,
                        "three" => EnergyScale::Three,
                        _ => EnergyScale::Five,
                    };
                    out.push(EnergyReport {
                        id: parse_uuid(&id_str)?,
                        recorded_at: row.get(1)?,
                        level: u8::try_from(level_i64).unwrap_or(1),
                        scale,
                        note: row.get(4)?,
                    });
                }
                Ok(out)
            })
            .await
    }
}

#[derive(Clone)]
pub struct NudgeRepository {
    db: Database,
}

impl NudgeRepository {
    #[must_use]
    pub fn new(db: Database) -> Self {
        Self { db }
    }

    pub async fn record_nudge(
        &self,
        id: Uuid,
        triggered_at: i64,
        kind: String,
        task_id: Option<Uuid>,
    ) -> Result<(), StorageError> {
        self.db
            .write_local(move |conn| {
                conn.execute(
                    "INSERT INTO nudge_event(id, triggered_at, kind, task_id)
                     VALUES (?1, ?2, ?3, ?4)",
                    params![
                        id.to_string(),
                        triggered_at,
                        kind,
                        task_id.map(|u| u.to_string())
                    ],
                )?;
                Ok(())
            })
            .await
    }

    pub async fn update_outcome(
        &self,
        id: Uuid,
        outcome: String,
        responded_at: i64,
    ) -> Result<(), StorageError> {
        self.db
            .write_local(move |conn| {
                conn.execute(
                    "UPDATE nudge_event SET outcome = ?2, responded_at = ?3 WHERE id = ?1",
                    params![id.to_string(), outcome, responded_at],
                )?;
                Ok(())
            })
            .await
    }

    pub async fn record_intervention(
        &self,
        id: Uuid,
        triggered_at: i64,
        kind: String,
        task_id: Option<Uuid>,
        accepted: Option<bool>,
        notes: Option<String>,
    ) -> Result<(), StorageError> {
        self.db
            .write_local(move |conn| {
                conn.execute(
                    "INSERT INTO intervention(id, triggered_at, kind, task_id, accepted, notes)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                    params![
                        id.to_string(),
                        triggered_at,
                        kind,
                        task_id.map(|u| u.to_string()),
                        accepted.map(i64::from),
                        notes
                    ],
                )?;
                Ok(())
            })
            .await
    }
}

#[derive(Clone)]
pub struct LlmTelemetryRepository {
    db: Database,
}

impl LlmTelemetryRepository {
    #[must_use]
    pub fn new(db: Database) -> Self {
        Self { db }
    }

    pub async fn record_llm_call(
        &self,
        id: Uuid,
        called_at: i64,
        record: LlmUsageRecord,
    ) -> Result<(), StorageError> {
        let feature_str = serde_json::to_value(record.feature)?
            .as_str()
            .unwrap_or("deterministic_tier0")
            .to_string();
        let tier_str = match record.tier {
            AiTier::T0 => "t0",
            AiTier::T1 => "t1",
            AiTier::T2 => "t2",
            AiTier::T3 => "t3",
        };
        let stop_str = match record.stop_reason {
            StopReason::Stop => "stop",
            StopReason::MaxTokens => "max_tokens",
            StopReason::Refusal => "refusal",
            StopReason::Other => "other",
        };
        self.db
            .write_local(move |conn| {
                conn.execute(
                    "INSERT INTO llm_call(id, called_at, feature, tier, model, prompt_ver, input_tokens, cached_tokens, output_tokens, latency_ms, estimated_cost_usd, stop_reason, truncated, valid, retried)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)",
                    params![
                        id.to_string(),
                        called_at,
                        feature_str,
                        tier_str,
                        record.model,
                        record.prompt_ver,
                        i64::from(record.input_tokens),
                        i64::from(record.cached_tokens),
                        i64::from(record.output_tokens),
                        i64::try_from(record.latency_ms).unwrap_or(i64::MAX),
                        record.estimated_cost_usd,
                        stop_str,
                        i64::from(record.truncated),
                        i64::from(record.valid),
                        i64::from(record.retried),
                    ],
                )?;
                Ok(())
            })
            .await
    }

    pub async fn cache_get(
        &self,
        cache_key: String,
        now_ms: i64,
    ) -> Result<Option<String>, StorageError> {
        self.db
            .read(move |conn| {
                let hit: Option<String> = conn
                    .query_row(
                        "SELECT response_json FROM response_cache WHERE cache_key = ?1 AND expires_at > ?2",
                        params![cache_key, now_ms],
                        |row| row.get(0),
                    )
                    .optional()?;
                Ok(hit)
            })
            .await
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn cache_put(
        &self,
        cache_key: String,
        feature: FeatureId,
        prompt_ver: String,
        profile_hash: String,
        normalized_input_hash: String,
        granularity: Option<u8>,
        response_json: String,
        created_at: i64,
        expires_at: i64,
    ) -> Result<(), StorageError> {
        let feature_str = serde_json::to_value(feature)?
            .as_str()
            .unwrap_or("deterministic_tier0")
            .to_string();
        self.db
            .write_local(move |conn| {
                conn.execute(
                    "INSERT INTO response_cache(cache_key, feature, prompt_ver, profile_hash, normalized_input_hash, granularity, response_json, created_at, expires_at)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
                     ON CONFLICT(cache_key) DO UPDATE SET
                         response_json = excluded.response_json,
                         created_at    = excluded.created_at,
                         expires_at    = excluded.expires_at",
                    params![
                        cache_key,
                        feature_str,
                        prompt_ver,
                        profile_hash,
                        normalized_input_hash,
                        granularity.map(i64::from),
                        response_json,
                        created_at,
                        expires_at,
                    ],
                )?;
                Ok(())
            })
            .await
    }

    pub async fn purge_expired_cache(&self, now_ms: i64) -> Result<usize, StorageError> {
        self.db
            .write_local(move |conn| {
                let count = conn.execute(
                    "DELETE FROM response_cache WHERE expires_at <= ?1",
                    params![now_ms],
                )?;
                Ok(count)
            })
            .await
    }
}

#[derive(Clone)]
pub struct MlStateRepository {
    db: Database,
}

impl MlStateRepository {
    #[must_use]
    pub fn new(db: Database) -> Self {
        Self { db }
    }

    pub async fn get(&self, model_key: String) -> Result<Option<String>, StorageError> {
        self.db
            .read(move |conn| {
                let val: Option<String> = conn
                    .query_row(
                        "SELECT state_json FROM ml_state WHERE model_key = ?1",
                        params![model_key],
                        |row| row.get(0),
                    )
                    .optional()?;
                Ok(val)
            })
            .await
    }

    pub async fn upsert(
        &self,
        model_key: String,
        state_json: String,
        updated_at: i64,
    ) -> Result<(), StorageError> {
        self.db
            .write_local(move |conn| {
                conn.execute(
                    "INSERT INTO ml_state(model_key, state_json, updated_at)
                     VALUES (?1, ?2, ?3)
                     ON CONFLICT(model_key) DO UPDATE SET
                         state_json = excluded.state_json,
                         updated_at = excluded.updated_at",
                    params![model_key, state_json, updated_at],
                )?;
                Ok(())
            })
            .await
    }
}
