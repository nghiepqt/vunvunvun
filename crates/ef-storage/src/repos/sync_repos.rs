use std::collections::BTreeMap;
use std::str::FromStr as _;

use ef_core::{
    DuePrecision, InboxItem, InboxSource, InboxStatus, Note, NoteKind, Origin, Project,
    SyncMetadata, Task, TaskCategory, TaskStatus, Transcript, UserAcceptedProof, UserProfile,
    UserProfileRecord, new_entity_id,
};
use ef_oplog::{EntityType, SyncMutation};
use rusqlite::{OptionalExtension as _, Row, params};
use serde_json::{Map, Value};
use uuid::Uuid;

use crate::db::{Database, current_unix_ms};
use crate::error::StorageError;

fn parse_uuid(raw: &str) -> Result<Uuid, StorageError> {
    Uuid::parse_str(raw).map_err(|e| StorageError::InvalidUuid(format!("{raw}: {e}")))
}

fn parse_opt_uuid(raw: Option<String>) -> Result<Option<Uuid>, StorageError> {
    raw.as_deref().map(parse_uuid).transpose()
}

fn row_sync_meta(row: &Row<'_>, offset: usize) -> rusqlite::Result<SyncMetadata> {
    Ok(SyncMetadata {
        updated_at: row.get(offset)?,
        deleted_at: row.get(offset + 1)?,
        device_id: row.get(offset + 2)?,
        hlc: row.get(offset + 3)?,
    })
}

fn map_inbox_row(row: &Row<'_>) -> Result<InboxItem, StorageError> {
    let id_str: String = row.get(0)?;
    let source_str: String = row.get(2)?;
    let status_str: String = row.get(3)?;
    Ok(InboxItem {
        id: parse_uuid(&id_str)?,
        raw_text: row.get(1)?,
        source: InboxSource::from_str(&source_str)?,
        status: InboxStatus::from_str(&status_str)?,
        created_at: row.get(4)?,
        processed_at: row.get(5)?,
        sync: row_sync_meta(row, 6)?,
    })
}

fn map_project_row(row: &Row<'_>) -> Result<Project, StorageError> {
    let id_str: String = row.get(0)?;
    let archived_i64: i64 = row.get(2)?;
    Ok(Project {
        id: parse_uuid(&id_str)?,
        name: row.get(1)?,
        archived: archived_i64 != 0,
        created_at: row.get(3)?,
        sync: row_sync_meta(row, 4)?,
    })
}

fn map_task_row(row: &Row<'_>) -> Result<Task, StorageError> {
    let id_str: String = row.get(0)?;
    let project_id_str: Option<String> = row.get(1)?;
    let category_str: String = row.get(3)?;
    let parent_id_str: Option<String> = row.get(4)?;
    let granularity_opt: Option<i64> = row.get(7)?;
    let status_str: String = row.get(8)?;
    let due_prec_str: Option<String> = row.get(10)?;
    let origin_str: String = row.get(17)?;

    Ok(Task {
        id: parse_uuid(&id_str)?,
        project_id: parse_opt_uuid(project_id_str)?,
        title: row.get(2)?,
        category: TaskCategory::from_str(&category_str)?,
        parent_id: parse_opt_uuid(parent_id_str)?,
        position: row.get(5)?,
        position_device_id: row.get(6)?,
        granularity: granularity_opt.and_then(|g| u8::try_from(g).ok()),
        status: TaskStatus::from_str(&status_str)?,
        due_at: row.get(9)?,
        due_precision: due_prec_str
            .as_deref()
            .map(DuePrecision::from_str)
            .transpose()?,
        urgency: row.get(11)?,
        target_app_id: row.get(12)?,
        est_p50_sec: row.get(13)?,
        est_p80_sec: row.get(14)?,
        est_p90_sec: row.get(15)?,
        actual_sec: row.get(16)?,
        origin: Origin::from_str(&origin_str)?,
        origin_model: row.get(18)?,
        origin_prompt_ver: row.get(19)?,
        accepted_at: row.get(20)?,
        created_at: row.get(21)?,
        sync: row_sync_meta(row, 22)?,
    })
}

fn map_note_row(row: &Row<'_>) -> Result<Note, StorageError> {
    let id_str: String = row.get(0)?;
    let kind_str: String = row.get(1)?;
    let answered_i64: i64 = row.get(4)?;
    let origin_str: String = row.get(5)?;
    Ok(Note {
        id: parse_uuid(&id_str)?,
        kind: NoteKind::from_str(&kind_str)?,
        text: row.get(2)?,
        starts_at: row.get(3)?,
        answered: answered_i64 != 0,
        origin: Origin::from_str(&origin_str)?,
        origin_model: row.get(6)?,
        origin_prompt_ver: row.get(7)?,
        accepted_at: row.get(8)?,
        created_at: row.get(9)?,
        sync: row_sync_meta(row, 10)?,
    })
}

fn map_transcript_row(row: &Row<'_>) -> Result<Transcript, StorageError> {
    let id_str: String = row.get(0)?;
    let consent_i64: i64 = row.get(4)?;
    Ok(Transcript {
        id: parse_uuid(&id_str)?,
        text: row.get(1)?,
        started_at: row.get(2)?,
        duration_sec: row.get(3)?,
        consent_ack: consent_i64 != 0,
        created_at: row.get(5)?,
        sync: row_sync_meta(row, 6)?,
    })
}

#[derive(Clone)]
pub struct InboxRepository {
    db: Database,
}

impl InboxRepository {
    #[must_use]
    pub fn new(db: Database) -> Self {
        Self { db }
    }

    pub async fn create(
        &self,
        id: Uuid,
        raw_text: String,
        source: InboxSource,
    ) -> Result<(), StorageError> {
        self.db
            .apply_sync(SyncMutation::CreateInboxItem {
                id,
                raw_text,
                source,
            })
            .await?;
        Ok(())
    }

    pub async fn get(&self, id: Uuid) -> Result<Option<InboxItem>, StorageError> {
        let id_str = id.to_string();
        self.db
            .read(move |conn| {
                let mut stmt = conn.prepare(
                    "SELECT id, raw_text, source, status, created_at, processed_at, updated_at, deleted_at, device_id, hlc
                     FROM inbox_item WHERE id = ?1",
                )?;
                let mut rows = stmt.query(params![id_str])?;
                match rows.next()? {
                    Some(row) => Ok(Some(map_inbox_row(row)?)),
                    None => Ok(None),
                }
            })
            .await
    }

    pub async fn list_active(
        &self,
        status: Option<InboxStatus>,
    ) -> Result<Vec<InboxItem>, StorageError> {
        let status_filter = status.map(|s| s.as_str().to_string());
        self.db
            .read(move |conn| {
                let sql = if status_filter.is_some() {
                    "SELECT id, raw_text, source, status, created_at, processed_at, updated_at, deleted_at, device_id, hlc
                     FROM inbox_item WHERE deleted_at IS NULL AND status = ?1 ORDER BY created_at DESC"
                } else {
                    "SELECT id, raw_text, source, status, created_at, processed_at, updated_at, deleted_at, device_id, hlc
                     FROM inbox_item WHERE deleted_at IS NULL ORDER BY created_at DESC"
                };
                let mut stmt = conn.prepare(sql)?;
                let mut out = Vec::new();
                let mut rows = if let Some(st) = status_filter {
                    stmt.query(params![st])?
                } else {
                    stmt.query([])?
                };
                while let Some(row) = rows.next()? {
                    out.push(map_inbox_row(row)?);
                }
                Ok(out)
            })
            .await
    }

    pub async fn set_status(
        &self,
        id: Uuid,
        status: InboxStatus,
        processed_at: Option<i64>,
    ) -> Result<(), StorageError> {
        self.db
            .apply_sync(SyncMutation::SetField {
                entity_type: EntityType::InboxItem,
                entity_id: id,
                field: "status".to_string(),
                value_json: serde_json::json!(status.as_str()),
            })
            .await?;
        if processed_at.is_some() {
            self.db
                .apply_sync(SyncMutation::SetField {
                    entity_type: EntityType::InboxItem,
                    entity_id: id,
                    field: "processed_at".to_string(),
                    value_json: serde_json::json!(processed_at),
                })
                .await?;
        }
        Ok(())
    }

    pub async fn delete(&self, id: Uuid) -> Result<(), StorageError> {
        self.db
            .apply_sync(SyncMutation::Delete {
                entity_type: EntityType::InboxItem,
                entity_id: id,
            })
            .await?;
        Ok(())
    }
}

#[derive(Clone)]
pub struct ProjectRepository {
    db: Database,
}

impl ProjectRepository {
    #[must_use]
    pub fn new(db: Database) -> Self {
        Self { db }
    }

    pub async fn create(&self, id: Uuid, name: String) -> Result<(), StorageError> {
        self.db
            .apply_sync(SyncMutation::CreateProject { id, name })
            .await?;
        Ok(())
    }

    pub async fn get(&self, id: Uuid) -> Result<Option<Project>, StorageError> {
        let id_str = id.to_string();
        self.db
            .read(move |conn| {
                let mut stmt = conn.prepare(
                    "SELECT id, name, archived, created_at, updated_at, deleted_at, device_id, hlc
                     FROM project WHERE id = ?1",
                )?;
                let mut rows = stmt.query(params![id_str])?;
                match rows.next()? {
                    Some(row) => Ok(Some(map_project_row(row)?)),
                    None => Ok(None),
                }
            })
            .await
    }

    pub async fn list_active(&self, include_archived: bool) -> Result<Vec<Project>, StorageError> {
        self.db
            .read(move |conn| {
                let sql = if include_archived {
                    "SELECT id, name, archived, created_at, updated_at, deleted_at, device_id, hlc
                     FROM project WHERE deleted_at IS NULL ORDER BY created_at ASC"
                } else {
                    "SELECT id, name, archived, created_at, updated_at, deleted_at, device_id, hlc
                     FROM project WHERE deleted_at IS NULL AND archived = 0 ORDER BY created_at ASC"
                };
                let mut stmt = conn.prepare(sql)?;
                let mut rows = stmt.query([])?;
                let mut out = Vec::new();
                while let Some(row) = rows.next()? {
                    out.push(map_project_row(row)?);
                }
                Ok(out)
            })
            .await
    }

    pub async fn rename(&self, id: Uuid, name: String) -> Result<(), StorageError> {
        self.db
            .apply_sync(SyncMutation::SetField {
                entity_type: EntityType::Project,
                entity_id: id,
                field: "name".to_string(),
                value_json: serde_json::json!(name),
            })
            .await?;
        Ok(())
    }

    pub async fn set_archived(&self, id: Uuid, archived: bool) -> Result<(), StorageError> {
        self.db
            .apply_sync(SyncMutation::SetField {
                entity_type: EntityType::Project,
                entity_id: id,
                field: "archived".to_string(),
                value_json: serde_json::json!(archived),
            })
            .await?;
        Ok(())
    }

    pub async fn delete(&self, id: Uuid) -> Result<(), StorageError> {
        self.db
            .apply_sync(SyncMutation::Delete {
                entity_type: EntityType::Project,
                entity_id: id,
            })
            .await?;
        Ok(())
    }
}

#[derive(Debug, Clone)]
pub struct CreateTaskInput {
    pub id: Uuid,
    pub project_id: Option<Uuid>,
    pub title: String,
    pub category: TaskCategory,
    pub parent_id: Option<Uuid>,
    pub position: String,
    pub granularity: Option<u8>,
    pub status: TaskStatus,
    pub due_at: Option<i64>,
    pub due_precision: Option<DuePrecision>,
    pub urgency: Option<String>,
    pub target_app_id: Option<String>,
    pub est_p50_sec: Option<i64>,
    pub est_p80_sec: Option<i64>,
    pub est_p90_sec: Option<i64>,
    pub origin: Origin,
    pub origin_model: Option<String>,
    pub origin_prompt_ver: Option<String>,
}

#[derive(Clone)]
pub struct TaskRepository {
    db: Database,
}

impl TaskRepository {
    #[must_use]
    pub fn new(db: Database) -> Self {
        Self { db }
    }

    pub async fn create(&self, input: CreateTaskInput) -> Result<(), StorageError> {
        self.db
            .apply_sync(SyncMutation::CreateTask {
                id: input.id,
                project_id: input.project_id,
                title: input.title,
                category: input.category,
                parent_id: input.parent_id,
                position: input.position,
                granularity: input.granularity,
                status: input.status,
                due_at: input.due_at,
                due_precision: input.due_precision,
                urgency: input.urgency,
                target_app_id: input.target_app_id,
                est_p50_sec: input.est_p50_sec,
                est_p80_sec: input.est_p80_sec,
                est_p90_sec: input.est_p90_sec,
                origin: input.origin,
                origin_model: input.origin_model,
                origin_prompt_ver: input.origin_prompt_ver,
            })
            .await?;
        Ok(())
    }

    pub async fn get(&self, id: Uuid) -> Result<Option<Task>, StorageError> {
        let id_str = id.to_string();
        self.db
            .read(move |conn| {
                let mut stmt = conn.prepare(
                    "SELECT id, project_id, title, category, parent_id, position, position_device_id, granularity, status, due_at, due_precision, urgency, target_app_id, est_p50_sec, est_p80_sec, est_p90_sec, actual_sec, origin, origin_model, origin_prompt_ver, accepted_at, created_at, updated_at, deleted_at, device_id, hlc
                     FROM task WHERE id = ?1",
                )?;
                let mut rows = stmt.query(params![id_str])?;
                match rows.next()? {
                    Some(row) => Ok(Some(map_task_row(row)?)),
                    None => Ok(None),
                }
            })
            .await
    }

    pub async fn list_by_parent(&self, parent_id: Option<Uuid>) -> Result<Vec<Task>, StorageError> {
        let parent_str = parent_id.map(|u| u.to_string());
        self.db
            .read(move |conn| {
                let sql = if parent_str.is_some() {
                    "SELECT id, project_id, title, category, parent_id, position, position_device_id, granularity, status, due_at, due_precision, urgency, target_app_id, est_p50_sec, est_p80_sec, est_p90_sec, actual_sec, origin, origin_model, origin_prompt_ver, accepted_at, created_at, updated_at, deleted_at, device_id, hlc
                     FROM task WHERE deleted_at IS NULL AND parent_id = ?1
                     ORDER BY position ASC, position_device_id ASC, id ASC"
                } else {
                    "SELECT id, project_id, title, category, parent_id, position, position_device_id, granularity, status, due_at, due_precision, urgency, target_app_id, est_p50_sec, est_p80_sec, est_p90_sec, actual_sec, origin, origin_model, origin_prompt_ver, accepted_at, created_at, updated_at, deleted_at, device_id, hlc
                     FROM task WHERE deleted_at IS NULL AND parent_id IS NULL
                     ORDER BY position ASC, position_device_id ASC, id ASC"
                };
                let mut stmt = conn.prepare(sql)?;
                let mut out = Vec::new();
                let mut rows = if let Some(pid) = parent_str {
                    stmt.query(params![pid])?
                } else {
                    stmt.query([])?
                };
                while let Some(row) = rows.next()? {
                    out.push(map_task_row(row)?);
                }
                Ok(out)
            })
            .await
    }

    pub async fn list_by_status(&self, status: TaskStatus) -> Result<Vec<Task>, StorageError> {
        let st = status.as_str().to_string();
        self.db
            .read(move |conn| {
                let mut stmt = conn.prepare(
                    "SELECT id, project_id, title, category, parent_id, position, position_device_id, granularity, status, due_at, due_precision, urgency, target_app_id, est_p50_sec, est_p80_sec, est_p90_sec, actual_sec, origin, origin_model, origin_prompt_ver, accepted_at, created_at, updated_at, deleted_at, device_id, hlc
                     FROM task WHERE deleted_at IS NULL AND status = ?1
                     ORDER BY position ASC, position_device_id ASC, id ASC",
                )?;
                let mut rows = stmt.query(params![st])?;
                let mut out = Vec::new();
                while let Some(row) = rows.next()? {
                    out.push(map_task_row(row)?);
                }
                Ok(out)
            })
            .await
    }

    pub async fn set_title(&self, id: Uuid, title: String) -> Result<(), StorageError> {
        self.set_field(id, "title", serde_json::json!(title)).await
    }

    pub async fn set_status(&self, id: Uuid, status: TaskStatus) -> Result<(), StorageError> {
        self.set_field(id, "status", serde_json::json!(status.as_str()))
            .await
    }

    pub async fn set_position(&self, id: Uuid, position: String) -> Result<(), StorageError> {
        self.set_field(id, "position", serde_json::json!(position))
            .await
    }

    pub async fn set_due_date(
        &self,
        id: Uuid,
        due_at: Option<i64>,
        due_precision: Option<DuePrecision>,
    ) -> Result<(), StorageError> {
        self.set_field(id, "due_at", serde_json::json!(due_at))
            .await?;
        self.set_field(
            id,
            "due_precision",
            serde_json::json!(due_precision.map(DuePrecision::as_str)),
        )
        .await
    }

    pub async fn record_actual_sec(&self, id: Uuid, actual_sec: i64) -> Result<(), StorageError> {
        self.set_field(id, "actual_sec", serde_json::json!(actual_sec))
            .await
    }

    pub async fn accept(
        &self,
        id: Uuid,
        accepted_at_ms: i64,
        proof: UserAcceptedProof,
    ) -> Result<(), StorageError> {
        self.db
            .apply_sync(SyncMutation::AcceptEntity {
                entity_type: EntityType::Task,
                entity_id: id,
                accepted_at_ms,
                proof,
            })
            .await?;
        Ok(())
    }

    pub async fn delete(&self, id: Uuid) -> Result<(), StorageError> {
        self.db
            .apply_sync(SyncMutation::Delete {
                entity_type: EntityType::Task,
                entity_id: id,
            })
            .await?;
        Ok(())
    }

    async fn set_field(
        &self,
        id: Uuid,
        field: &str,
        value_json: Value,
    ) -> Result<(), StorageError> {
        self.db
            .apply_sync(SyncMutation::SetField {
                entity_type: EntityType::Task,
                entity_id: id,
                field: field.to_string(),
                value_json,
            })
            .await?;
        Ok(())
    }
}

#[derive(Debug, Clone)]
pub struct CreateNoteInput {
    pub id: Uuid,
    pub kind: NoteKind,
    pub text: String,
    pub starts_at: Option<i64>,
    pub origin: Origin,
    pub origin_model: Option<String>,
    pub origin_prompt_ver: Option<String>,
}

#[derive(Clone)]
pub struct NoteRepository {
    db: Database,
}

impl NoteRepository {
    #[must_use]
    pub fn new(db: Database) -> Self {
        Self { db }
    }

    pub async fn create(&self, input: CreateNoteInput) -> Result<(), StorageError> {
        self.db
            .apply_sync(SyncMutation::CreateNote {
                id: input.id,
                kind: input.kind,
                text: input.text,
                starts_at: input.starts_at,
                origin: input.origin,
                origin_model: input.origin_model,
                origin_prompt_ver: input.origin_prompt_ver,
            })
            .await?;
        Ok(())
    }

    pub async fn get(&self, id: Uuid) -> Result<Option<Note>, StorageError> {
        let id_str = id.to_string();
        self.db
            .read(move |conn| {
                let mut stmt = conn.prepare(
                    "SELECT id, kind, text, starts_at, answered, origin, origin_model, origin_prompt_ver, accepted_at, created_at, updated_at, deleted_at, device_id, hlc
                     FROM note WHERE id = ?1",
                )?;
                let mut rows = stmt.query(params![id_str])?;
                match rows.next()? {
                    Some(row) => Ok(Some(map_note_row(row)?)),
                    None => Ok(None),
                }
            })
            .await
    }

    pub async fn list_active(&self, kind: Option<NoteKind>) -> Result<Vec<Note>, StorageError> {
        let kind_filter = kind.map(|k| k.as_str().to_string());
        self.db
            .read(move |conn| {
                let sql = if kind_filter.is_some() {
                    "SELECT id, kind, text, starts_at, answered, origin, origin_model, origin_prompt_ver, accepted_at, created_at, updated_at, deleted_at, device_id, hlc
                     FROM note WHERE deleted_at IS NULL AND kind = ?1 ORDER BY created_at DESC"
                } else {
                    "SELECT id, kind, text, starts_at, answered, origin, origin_model, origin_prompt_ver, accepted_at, created_at, updated_at, deleted_at, device_id, hlc
                     FROM note WHERE deleted_at IS NULL ORDER BY created_at DESC"
                };
                let mut stmt = conn.prepare(sql)?;
                let mut rows = if let Some(k) = kind_filter {
                    stmt.query(params![k])?
                } else {
                    stmt.query([])?
                };
                let mut out = Vec::new();
                while let Some(row) = rows.next()? {
                    out.push(map_note_row(row)?);
                }
                Ok(out)
            })
            .await
    }

    pub async fn mark_answered(&self, id: Uuid, answered: bool) -> Result<(), StorageError> {
        self.db
            .apply_sync(SyncMutation::SetField {
                entity_type: EntityType::Note,
                entity_id: id,
                field: "answered".to_string(),
                value_json: serde_json::json!(answered),
            })
            .await?;
        Ok(())
    }

    pub async fn accept(
        &self,
        id: Uuid,
        accepted_at_ms: i64,
        proof: UserAcceptedProof,
    ) -> Result<(), StorageError> {
        self.db
            .apply_sync(SyncMutation::AcceptEntity {
                entity_type: EntityType::Note,
                entity_id: id,
                accepted_at_ms,
                proof,
            })
            .await?;
        Ok(())
    }

    pub async fn delete(&self, id: Uuid) -> Result<(), StorageError> {
        self.db
            .apply_sync(SyncMutation::Delete {
                entity_type: EntityType::Note,
                entity_id: id,
            })
            .await?;
        Ok(())
    }
}

#[derive(Clone)]
pub struct TranscriptRepository {
    db: Database,
}

impl TranscriptRepository {
    #[must_use]
    pub fn new(db: Database) -> Self {
        Self { db }
    }

    pub async fn create(
        &self,
        id: Uuid,
        text: String,
        started_at: i64,
        duration_sec: i64,
        consent_ack: bool,
    ) -> Result<(), StorageError> {
        self.db
            .apply_sync(SyncMutation::CreateTranscript {
                id,
                text,
                started_at,
                duration_sec,
                consent_ack,
            })
            .await?;
        Ok(())
    }

    pub async fn get(&self, id: Uuid) -> Result<Option<Transcript>, StorageError> {
        let id_str = id.to_string();
        self.db
            .read(move |conn| {
                let mut stmt = conn.prepare(
                    "SELECT id, text, started_at, duration_sec, consent_ack, created_at, updated_at, deleted_at, device_id, hlc
                     FROM transcript WHERE id = ?1",
                )?;
                let mut rows = stmt.query(params![id_str])?;
                match rows.next()? {
                    Some(row) => Ok(Some(map_transcript_row(row)?)),
                    None => Ok(None),
                }
            })
            .await
    }

    pub async fn list_active(&self) -> Result<Vec<Transcript>, StorageError> {
        self.db
            .read(move |conn| {
                let mut stmt = conn.prepare(
                    "SELECT id, text, started_at, duration_sec, consent_ack, created_at, updated_at, deleted_at, device_id, hlc
                     FROM transcript WHERE deleted_at IS NULL ORDER BY started_at DESC",
                )?;
                let mut rows = stmt.query([])?;
                let mut out = Vec::new();
                while let Some(row) = rows.next()? {
                    out.push(map_transcript_row(row)?);
                }
                Ok(out)
            })
            .await
    }

    pub async fn delete(&self, id: Uuid) -> Result<(), StorageError> {
        self.db
            .apply_sync(SyncMutation::Delete {
                entity_type: EntityType::Transcript,
                entity_id: id,
            })
            .await?;
        Ok(())
    }
}

#[derive(Clone)]
pub struct UserProfileRepository {
    db: Database,
}

impl UserProfileRepository {
    #[must_use]
    pub fn new(db: Database) -> Self {
        Self { db }
    }

    pub async fn get(&self, id: Uuid) -> Result<Option<UserProfileRecord>, StorageError> {
        let id_str = id.to_string();
        self.db
            .read(move |conn| read_profile_by_id(conn, &id_str))
            .await
    }

    pub async fn get_or_init(&self) -> Result<UserProfileRecord, StorageError> {
        let existing = self
            .db
            .read(|conn| {
                let id_opt: Option<String> = conn
                    .query_row(
                        "SELECT id FROM user_profile WHERE deleted_at IS NULL ORDER BY updated_at ASC LIMIT 1",
                        [],
                        |row| row.get(0),
                    )
                    .optional()?;
                match id_opt {
                    Some(id) => read_profile_by_id(conn, &id),
                    None => Ok(None),
                }
            })
            .await?;

        if let Some(rec) = existing {
            return Ok(rec);
        }

        let id = new_entity_id();
        self.db
            .apply_sync_at(
                SyncMutation::CreateUserProfile {
                    id,
                    profile: UserProfile::default(),
                },
                current_unix_ms(),
            )
            .await?;

        self.get(id)
            .await?
            .ok_or_else(|| StorageError::NotFound(format!("user_profile {id}")))
    }

    pub async fn set_preference(
        &self,
        id: Uuid,
        key: impl Into<String>,
        value: Value,
    ) -> Result<UserProfileRecord, StorageError> {
        let mut fields = BTreeMap::new();
        fields.insert(key.into(), value);
        self.db
            .apply_sync(SyncMutation::SetUserProfileFields { id, fields })
            .await?;
        self.get(id)
            .await?
            .ok_or_else(|| StorageError::NotFound(format!("user_profile {id}")))
    }

    pub async fn update_profile(
        &self,
        id: Uuid,
        updated: &UserProfile,
    ) -> Result<UserProfileRecord, StorageError> {
        let current = self
            .get(id)
            .await?
            .ok_or_else(|| StorageError::NotFound(format!("user_profile {id}")))?;
        let current_val = serde_json::to_value(&current.profile)?;
        let next_val = serde_json::to_value(updated)?;

        let mut patch = BTreeMap::new();
        if let (Some(curr_obj), Some(next_obj)) = (current_val.as_object(), next_val.as_object()) {
            for (k, v) in next_obj {
                if curr_obj.get(k) != Some(v) {
                    patch.insert(k.clone(), v.clone());
                }
            }
        }

        if !patch.is_empty() {
            self.db
                .apply_sync(SyncMutation::SetUserProfileFields { id, fields: patch })
                .await?;
        }

        self.get(id)
            .await?
            .ok_or_else(|| StorageError::NotFound(format!("user_profile {id}")))
    }
}

fn read_profile_by_id(
    conn: &rusqlite::Connection,
    id_str: &str,
) -> Result<Option<UserProfileRecord>, StorageError> {
    struct RawProfileRow {
        id: String,
        json_str: String,
        profile_hash: String,
        cluster_id: String,
        updated_at: i64,
        deleted_at: Option<i64>,
        device_id: String,
        hlc: String,
    }

    let row_opt = conn
        .query_row(
            "SELECT id, json, profile_hash, cluster_id, updated_at, deleted_at, device_id, hlc
             FROM user_profile WHERE id = ?1",
            params![id_str],
            |r| {
                Ok(RawProfileRow {
                    id: r.get(0)?,
                    json_str: r.get(1)?,
                    profile_hash: r.get(2)?,
                    cluster_id: r.get(3)?,
                    updated_at: r.get(4)?,
                    deleted_at: r.get(5)?,
                    device_id: r.get(6)?,
                    hlc: r.get(7)?,
                })
            },
        )
        .optional()?;

    let Some(raw) = row_opt else {
        return Ok(None);
    };

    let current_map: Map<String, Value> = serde_json::from_str(&raw.json_str)?;
    let mut merged = serde_json::to_value(UserProfile::default())?;
    if let Some(obj) = merged.as_object_mut() {
        for (k, v) in current_map {
            obj.insert(k, v);
        }
    }
    let profile: UserProfile = serde_json::from_value(merged)?;
    Ok(Some(UserProfileRecord {
        id: parse_uuid(&raw.id)?,
        profile,
        profile_hash: raw.profile_hash,
        cluster_id: raw.cluster_id,
        sync: SyncMetadata {
            updated_at: raw.updated_at,
            deleted_at: raw.deleted_at,
            device_id: raw.device_id,
            hlc: raw.hlc,
        },
    }))
}
