use std::str::FromStr as _;

use ef_core::{
    DuePrecision, Hlc, InboxSource, InboxStatus, NoteKind, Origin, TaskCategory, TaskStatus,
    UserProfile,
};
use rusqlite::{OptionalExtension as _, Transaction, params};
use serde_json::{Map, Value, json};
use uuid::Uuid;

use crate::conn::SyncWriteConn;
use crate::mutation::{ApplyOutcome, EntityType, OpKind, OpLogError, OpRecord, SyncMutation};

pub struct OpLog<'a> {
    write_conn: &'a mut SyncWriteConn,
    hlc: &'a mut Hlc,
}

impl<'a> OpLog<'a> {
    pub fn new(write_conn: &'a mut SyncWriteConn, hlc: &'a mut Hlc) -> Self {
        Self { write_conn, hlc }
    }

    /// Loads `(device_id, last_ms, counter)` from `hlc_state`, initializing `device` and
    /// `hlc_state` on first launch.
    pub fn init_hlc(write_conn: &mut SyncWriteConn, now_ms: i64) -> Result<Hlc, OpLogError> {
        let clamped_now = now_ms.max(0);
        let conn = write_conn.conn_mut();
        let tx = conn.transaction()?;

        let existing: Option<(String, i64, u32)> = tx
            .query_row(
                "SELECT device_id, last_ms, counter FROM hlc_state WHERE id = 1",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .optional()?;

        let hlc = if let Some((device_id, last_ms, counter)) = existing {
            Hlc::new(device_id, last_ms, counter)
        } else {
            let existing_device: Option<String> = tx
                .query_row("SELECT id FROM device LIMIT 1", [], |row| row.get(0))
                .optional()?;
            let device_id = match existing_device {
                Some(id) => id,
                None => {
                    let id = Uuid::now_v7().to_string();
                    tx.execute(
                        "INSERT INTO device(id, name, created_at) VALUES (?1, ?2, ?3)",
                        params![id, "local-windows-device", clamped_now],
                    )?;
                    id
                }
            };
            tx.execute(
                "INSERT INTO hlc_state(id, device_id, last_ms, counter) VALUES (1, ?1, ?2, 0)",
                params![device_id, clamped_now],
            )?;
            Hlc::new(device_id, clamped_now, 0)
        };

        tx.commit()?;
        Ok(hlc)
    }

    /// Validates invariants, appends operation(s) to `op_log`, persists `hlc_state`,
    /// and materializes the target syncable table(s) inside one atomic SQLite transaction.
    pub fn apply(
        &mut self,
        mutation: SyncMutation,
        now_ms: i64,
    ) -> Result<Vec<OpRecord>, OpLogError> {
        validate_local_mutation(&mutation)?;

        let saved_hlc = self.hlc.clone();
        let ops = build_local_ops(mutation, self.hlc, now_ms)?;
        if ops.is_empty() {
            return Ok(ops);
        }

        let result: Result<(), OpLogError> = (|| {
            let tx = self.write_conn.conn_mut().transaction()?;
            persist_hlc_state(&tx, self.hlc)?;
            for op in &ops {
                insert_oplog_row(&tx, op, None)?;
                materialize_op(&tx, op)?;
            }
            tx.commit()?;
            Ok(())
        })();

        if result.is_err() {
            *self.hlc = saved_hlc;
        }
        result?;
        Ok(ops)
    }

    /// Applies a remote (or replayed) operation idempotently with bounded clock-drift
    /// enforcement and deterministic per-field conflict resolution.
    pub fn apply_remote(&mut self, op: &OpRecord, now_ms: i64) -> Result<ApplyOutcome, OpLogError> {
        if op.op == OpKind::Set
            && op.entity_type == EntityType::InboxItem
            && op.field.as_deref() == Some("raw_text")
        {
            return Err(OpLogError::ImmutableRawText);
        }

        let parsed = Hlc::parse(&op.hlc)?;
        let saved_hlc = self.hlc.clone();
        self.hlc.observe(parsed.ms, parsed.counter, now_ms)?;

        let result: Result<ApplyOutcome, OpLogError> = (|| {
            let tx = self.write_conn.conn_mut().transaction()?;
            let already_seen: Option<i64> = tx
                .query_row(
                    "SELECT seq FROM op_log WHERE op_id = ?1",
                    params![op.op_id.to_string()],
                    |row| row.get(0),
                )
                .optional()?;
            if already_seen.is_some() {
                return Ok(ApplyOutcome::Duplicate);
            }

            persist_hlc_state(&tx, self.hlc)?;
            insert_oplog_row(&tx, op, Some(now_ms.max(0)))?;
            materialize_op(&tx, op)?;
            tx.commit()?;
            Ok(ApplyOutcome::Applied)
        })();

        if result.is_err() {
            *self.hlc = saved_hlc;
        }
        result
    }

    /// Lists un-acknowledged operations in local sequence order.
    pub fn list_unsynced_ops(write_conn: &mut SyncWriteConn) -> Result<Vec<OpRecord>, OpLogError> {
        let conn = write_conn.conn();
        let mut stmt = conn.prepare(
            "SELECT op_id, hlc, device_id, entity_type, entity_id, op, field, value
             FROM op_log
             WHERE synced_at IS NULL
             ORDER BY seq ASC",
        )?;
        let rows = stmt.query_map([], |row| {
            let op_id_str: String = row.get(0)?;
            let hlc: String = row.get(1)?;
            let device_id: String = row.get(2)?;
            let entity_type_str: String = row.get(3)?;
            let entity_id_str: String = row.get(4)?;
            let op_str: String = row.get(5)?;
            let field: Option<String> = row.get(6)?;
            let value_str: Option<String> = row.get(7)?;
            Ok((
                op_id_str,
                hlc,
                device_id,
                entity_type_str,
                entity_id_str,
                op_str,
                field,
                value_str,
            ))
        })?;

        let mut out = Vec::new();
        for item in rows {
            let (op_id_str, hlc, device_id, entity_type_str, entity_id_str, op_str, field, val) =
                item?;
            let op_id = Uuid::parse_str(&op_id_str)
                .map_err(|e| OpLogError::InvalidValue(format!("invalid op_id UUID: {e}")))?;
            let entity_id = Uuid::parse_str(&entity_id_str)
                .map_err(|e| OpLogError::InvalidValue(format!("invalid entity_id UUID: {e}")))?;
            let entity_type = EntityType::from_str(&entity_type_str)?;
            let op = OpKind::from_str(&op_str)?;
            let value = val
                .as_deref()
                .map(serde_json::from_str::<Value>)
                .transpose()?;
            out.push(OpRecord {
                op_id,
                hlc,
                device_id,
                entity_type,
                entity_id,
                op,
                field,
                value,
            });
        }
        Ok(out)
    }
}

fn validate_local_mutation(mutation: &SyncMutation) -> Result<(), OpLogError> {
    match mutation {
        SyncMutation::SetField {
            entity_type: EntityType::InboxItem,
            field,
            ..
        } if field == "raw_text" => Err(OpLogError::ImmutableRawText),
        SyncMutation::SetField { field, .. } if field == "accepted_at" => {
            Err(OpLogError::AcceptedAtRequiresProof)
        }
        SyncMutation::AcceptEntity { entity_type, .. }
            if !matches!(entity_type, EntityType::Task | EntityType::Note) =>
        {
            Err(OpLogError::UnsupportedAcceptEntity(
                entity_type.as_str().to_string(),
            ))
        }
        SyncMutation::CreateTask {
            granularity: Some(g),
            ..
        } if !(1..=5).contains(g) => Err(OpLogError::InvalidValue(format!(
            "task granularity {g} out of 1..=5 range"
        ))),
        SyncMutation::CreateTranscript { duration_sec, .. } if *duration_sec < 0 => Err(
            OpLogError::InvalidValue("transcript duration_sec must be non-negative".to_string()),
        ),
        _ => Ok(()),
    }
}

fn build_local_ops(
    mutation: SyncMutation,
    hlc: &mut Hlc,
    now_ms: i64,
) -> Result<Vec<OpRecord>, OpLogError> {
    let device_id = hlc.device_id().to_string();
    let mut make_op = |entity_type: EntityType,
                       entity_id: Uuid,
                       op: OpKind,
                       field: Option<String>,
                       value: Option<Value>| OpRecord {
        op_id: Uuid::now_v7(),
        hlc: hlc.tick(now_ms),
        device_id: device_id.clone(),
        entity_type,
        entity_id,
        op,
        field,
        value,
    };

    let ops = match mutation {
        SyncMutation::CreateInboxItem {
            id,
            raw_text,
            source,
        } => vec![make_op(
            EntityType::InboxItem,
            id,
            OpKind::Create,
            None,
            Some(json!({
                "raw_text": raw_text,
                "source": source.as_str(),
                "status": InboxStatus::Pending.as_str(),
                "processed_at": Value::Null,
            })),
        )],
        SyncMutation::CreateProject { id, name } => vec![make_op(
            EntityType::Project,
            id,
            OpKind::Create,
            None,
            Some(json!({
                "name": name,
                "archived": false,
            })),
        )],
        SyncMutation::CreateTask {
            id,
            project_id,
            title,
            category,
            parent_id,
            position,
            granularity,
            status,
            due_at,
            due_precision,
            urgency,
            target_app_id,
            est_p50_sec,
            est_p80_sec,
            est_p90_sec,
            origin,
            origin_model,
            origin_prompt_ver,
        } => vec![make_op(
            EntityType::Task,
            id,
            OpKind::Create,
            None,
            Some(json!({
                "project_id": project_id.map(|u| u.to_string()),
                "title": title,
                "category": category.as_str(),
                "parent_id": parent_id.map(|u| u.to_string()),
                "position": position,
                "granularity": granularity,
                "status": status.as_str(),
                "due_at": due_at,
                "due_precision": due_precision.map(DuePrecision::as_str),
                "urgency": urgency,
                "target_app_id": target_app_id,
                "est_p50_sec": est_p50_sec,
                "est_p80_sec": est_p80_sec,
                "est_p90_sec": est_p90_sec,
                "actual_sec": 0,
                "origin": origin.as_str(),
                "origin_model": origin_model,
                "origin_prompt_ver": origin_prompt_ver,
            })),
        )],
        SyncMutation::CreateNote {
            id,
            kind,
            text,
            starts_at,
            origin,
            origin_model,
            origin_prompt_ver,
        } => vec![make_op(
            EntityType::Note,
            id,
            OpKind::Create,
            None,
            Some(json!({
                "kind": kind.as_str(),
                "text": text,
                "starts_at": starts_at,
                "answered": false,
                "origin": origin.as_str(),
                "origin_model": origin_model,
                "origin_prompt_ver": origin_prompt_ver,
            })),
        )],
        SyncMutation::CreateTranscript {
            id,
            text,
            started_at,
            duration_sec,
            consent_ack,
        } => vec![make_op(
            EntityType::Transcript,
            id,
            OpKind::Create,
            None,
            Some(json!({
                "text": text,
                "started_at": started_at,
                "duration_sec": duration_sec,
                "consent_ack": consent_ack,
            })),
        )],
        SyncMutation::CreateUserProfile { id, profile } => {
            let val = serde_json::to_value(&profile)?;
            vec![make_op(
                EntityType::UserProfile,
                id,
                OpKind::Create,
                None,
                Some(val),
            )]
        }
        SyncMutation::SetField {
            entity_type,
            entity_id,
            field,
            value_json,
        } => vec![make_op(
            entity_type,
            entity_id,
            OpKind::Set,
            Some(field),
            Some(value_json),
        )],
        SyncMutation::SetUserProfileFields { id, fields } => fields
            .into_iter()
            .map(|(k, v)| {
                let field_name = if k.starts_with("json.") {
                    k
                } else {
                    format!("json.{k}")
                };
                make_op(
                    EntityType::UserProfile,
                    id,
                    OpKind::Set,
                    Some(field_name),
                    Some(v),
                )
            })
            .collect(),
        SyncMutation::AcceptEntity {
            entity_type,
            entity_id,
            accepted_at_ms,
            proof: _,
        } => vec![make_op(
            entity_type,
            entity_id,
            OpKind::Set,
            Some("accepted_at".to_string()),
            Some(json!(accepted_at_ms)),
        )],
        SyncMutation::Delete {
            entity_type,
            entity_id,
        } => vec![make_op(entity_type, entity_id, OpKind::Delete, None, None)],
    };

    Ok(ops)
}

fn persist_hlc_state(tx: &Transaction<'_>, hlc: &Hlc) -> Result<(), OpLogError> {
    let (last_ms, counter) = hlc.state();
    tx.execute(
        "INSERT INTO hlc_state(id, device_id, last_ms, counter)
         VALUES (1, ?1, ?2, ?3)
         ON CONFLICT(id) DO UPDATE SET
             device_id = excluded.device_id,
             last_ms   = excluded.last_ms,
             counter   = excluded.counter",
        params![hlc.device_id(), last_ms, counter],
    )?;
    Ok(())
}

fn insert_oplog_row(
    tx: &Transaction<'_>,
    op: &OpRecord,
    synced_at: Option<i64>,
) -> Result<(), OpLogError> {
    let value_str = op.value.as_ref().map(serde_json::to_string).transpose()?;
    tx.execute(
        "INSERT INTO op_log(op_id, hlc, device_id, entity_type, entity_id, op, field, value, applied, synced_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, 1, ?9)",
        params![
            op.op_id.to_string(),
            op.hlc,
            op.device_id,
            op.entity_type.as_str(),
            op.entity_id.to_string(),
            op.op.as_str(),
            op.field,
            value_str,
            synced_at,
        ],
    )?;
    Ok(())
}

fn get_field_clock(
    tx: &Transaction<'_>,
    entity_type: EntityType,
    entity_id: &str,
    field: &str,
) -> Result<Option<String>, OpLogError> {
    let clock = tx
        .query_row(
            "SELECT hlc FROM field_clock WHERE entity_type = ?1 AND entity_id = ?2 AND field = ?3",
            params![entity_type.as_str(), entity_id, field],
            |row| row.get(0),
        )
        .optional()?;
    Ok(clock)
}

fn upsert_field_clock(
    tx: &Transaction<'_>,
    entity_type: EntityType,
    entity_id: &str,
    field: &str,
    hlc: &str,
) -> Result<(), OpLogError> {
    tx.execute(
        "INSERT INTO field_clock(entity_type, entity_id, field, hlc)
         VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT(entity_type, entity_id, field) DO UPDATE SET hlc = excluded.hlc",
        params![entity_type.as_str(), entity_id, field, hlc],
    )?;
    Ok(())
}

fn lww_should_apply(
    tx: &Transaction<'_>,
    entity_type: EntityType,
    entity_id: &str,
    field: &str,
    incoming_hlc: &str,
) -> Result<bool, OpLogError> {
    let existing = get_field_clock(tx, entity_type, entity_id, field)?;
    let wins = match existing {
        None => true,
        Some(prev) => incoming_hlc > prev.as_str(),
    };
    if wins {
        upsert_field_clock(tx, entity_type, entity_id, field, incoming_hlc)?;
    }
    Ok(wins)
}

fn update_row_hlc_if_newer(
    tx: &Transaction<'_>,
    table: &str,
    entity_id: &str,
    op_hlc: &str,
    op_device_id: &str,
    op_ms: i64,
) -> Result<(), OpLogError> {
    let sql = format!(
        "UPDATE {table} SET updated_at = ?2, device_id = ?3, hlc = ?4 WHERE id = ?1 AND hlc < ?4"
    );
    tx.execute(&sql, params![entity_id, op_ms, op_device_id, op_hlc])?;
    Ok(())
}

fn ensure_stub_row(
    tx: &Transaction<'_>,
    entity_type: EntityType,
    entity_id: &str,
) -> Result<(), OpLogError> {
    match entity_type {
        EntityType::InboxItem => {
            tx.execute(
                "INSERT OR IGNORE INTO inbox_item(id, raw_text, source, status, created_at, processed_at, updated_at, deleted_at, device_id, hlc)
                 VALUES (?1, '', 'hotkey', 'pending', 0, NULL, 0, NULL, '', '')",
                params![entity_id],
            )?;
        }
        EntityType::Project => {
            tx.execute(
                "INSERT OR IGNORE INTO project(id, name, archived, created_at, updated_at, deleted_at, device_id, hlc)
                 VALUES (?1, '', 0, 0, 0, NULL, '', '')",
                params![entity_id],
            )?;
        }
        EntityType::Task => {
            tx.execute(
                "INSERT OR IGNORE INTO task(id, project_id, title, category, parent_id, position, position_device_id, granularity, status, due_at, due_precision, urgency, target_app_id, est_p50_sec, est_p80_sec, est_p90_sec, actual_sec, origin, origin_model, origin_prompt_ver, accepted_at, created_at, updated_at, deleted_at, device_id, hlc)
                 VALUES (?1, NULL, '', 'general', NULL, 'V', '', NULL, 'open', NULL, NULL, NULL, NULL, NULL, NULL, NULL, 0, 'user', NULL, NULL, NULL, 0, 0, NULL, '', '')",
                params![entity_id],
            )?;
        }
        EntityType::Note => {
            tx.execute(
                "INSERT OR IGNORE INTO note(id, kind, text, starts_at, answered, origin, origin_model, origin_prompt_ver, accepted_at, created_at, updated_at, deleted_at, device_id, hlc)
                 VALUES (?1, 'note', '', NULL, 0, 'user', NULL, NULL, NULL, 0, 0, NULL, '', '')",
                params![entity_id],
            )?;
        }
        EntityType::Transcript => {
            tx.execute(
                "INSERT OR IGNORE INTO transcript(id, text, started_at, duration_sec, consent_ack, created_at, updated_at, deleted_at, device_id, hlc)
                 VALUES (?1, '', 0, 0, 0, 0, 0, NULL, '', '')",
                params![entity_id],
            )?;
        }
        EntityType::UserProfile => {
            let defaults = UserProfile::default();
            tx.execute(
                "INSERT OR IGNORE INTO user_profile(id, json, profile_hash, cluster_id, updated_at, deleted_at, device_id, hlc)
                 VALUES (?1, '{}', ?2, ?3, 0, NULL, '', '')",
                params![entity_id, defaults.profile_hash(), defaults.cluster_id()],
            )?;
        }
    }
    Ok(())
}

fn materialize_op(tx: &Transaction<'_>, op: &OpRecord) -> Result<(), OpLogError> {
    let parsed = Hlc::parse(&op.hlc)?;
    let op_ms = parsed.ms;
    let entity_id = op.entity_id.to_string();

    ensure_stub_row(tx, op.entity_type, &entity_id)?;

    match op.op {
        OpKind::Delete => {
            if lww_should_apply(tx, op.entity_type, &entity_id, "deleted_at", &op.hlc)? {
                let sql = format!(
                    "UPDATE {} SET deleted_at = ?2 WHERE id = ?1",
                    op.entity_type.as_str()
                );
                tx.execute(&sql, params![entity_id, op_ms])?;
            }
            update_row_hlc_if_newer(
                tx,
                op.entity_type.as_str(),
                &entity_id,
                &op.hlc,
                &op.device_id,
                op_ms,
            )?;
            return Ok(());
        }
        OpKind::Create => {
            let obj = op
                .value
                .as_ref()
                .and_then(Value::as_object)
                .ok_or_else(|| OpLogError::InvalidValue("create op missing JSON object".into()))?;
            if op.entity_type != EntityType::UserProfile
                && lww_should_apply(tx, op.entity_type, &entity_id, "created_at", &op.hlc)?
            {
                let sql = format!(
                    "UPDATE {} SET created_at = ?2 WHERE id = ?1",
                    op.entity_type.as_str()
                );
                tx.execute(&sql, params![entity_id, op_ms])?;
            }
            if op.entity_type == EntityType::UserProfile {
                apply_user_profile_map(tx, &entity_id, &op.hlc, obj)?;
            } else {
                for (field, val) in obj {
                    apply_entity_field(
                        tx,
                        op.entity_type,
                        &entity_id,
                        &op.hlc,
                        &op.device_id,
                        field,
                        val,
                    )?;
                }
            }
        }
        OpKind::Set => {
            let field = op
                .field
                .as_deref()
                .ok_or_else(|| OpLogError::InvalidValue("set op missing field name".into()))?;
            let val = op
                .value
                .as_ref()
                .ok_or_else(|| OpLogError::InvalidValue("set op missing value".into()))?;
            if op.entity_type == EntityType::UserProfile {
                let key = field
                    .strip_prefix("json.")
                    .ok_or_else(|| OpLogError::UnknownField {
                        entity_type: "user_profile".to_string(),
                        field: field.to_string(),
                    })?;
                if key.is_empty() {
                    return Err(OpLogError::UnknownField {
                        entity_type: "user_profile".to_string(),
                        field: field.to_string(),
                    });
                }
                let mut patch = Map::new();
                patch.insert(key.to_string(), val.clone());
                apply_user_profile_map(tx, &entity_id, &op.hlc, &patch)?;
            } else {
                apply_entity_field(
                    tx,
                    op.entity_type,
                    &entity_id,
                    &op.hlc,
                    &op.device_id,
                    field,
                    val,
                )?;
            }
        }
    }

    update_row_hlc_if_newer(
        tx,
        op.entity_type.as_str(),
        &entity_id,
        &op.hlc,
        &op.device_id,
        op_ms,
    )?;
    Ok(())
}

fn apply_user_profile_map(
    tx: &Transaction<'_>,
    entity_id: &str,
    op_hlc: &str,
    patch: &Map<String, Value>,
) -> Result<(), OpLogError> {
    let raw_json: String = tx.query_row(
        "SELECT json FROM user_profile WHERE id = ?1",
        params![entity_id],
        |row| row.get(0),
    )?;
    let mut current_map: Map<String, Value> = serde_json::from_str(&raw_json)?;
    let mut changed = false;

    for (key, value) in patch {
        let clock_field = format!("json.{key}");
        if lww_should_apply(tx, EntityType::UserProfile, entity_id, &clock_field, op_hlc)? {
            current_map.insert(key.clone(), value.clone());
            changed = true;
        }
    }

    if changed {
        let mut merged = serde_json::to_value(UserProfile::default())?;
        if let Some(merged_obj) = merged.as_object_mut() {
            for (k, v) in &current_map {
                merged_obj.insert(k.clone(), v.clone());
            }
        }
        let profile: UserProfile = serde_json::from_value(merged)?;
        let json_str = serde_json::to_string(&current_map)?;
        tx.execute(
            "UPDATE user_profile SET json = ?2, profile_hash = ?3, cluster_id = ?4 WHERE id = ?1",
            params![
                entity_id,
                json_str,
                profile.profile_hash(),
                profile.cluster_id()
            ],
        )?;
    }

    Ok(())
}

fn apply_entity_field(
    tx: &Transaction<'_>,
    entity_type: EntityType,
    entity_id: &str,
    op_hlc: &str,
    op_device_id: &str,
    field: &str,
    val: &Value,
) -> Result<(), OpLogError> {
    match entity_type {
        EntityType::InboxItem => match field {
            "raw_text" => {
                let s = req_str(val, field)?;
                if lww_should_apply(tx, entity_type, entity_id, field, op_hlc)? {
                    tx.execute(
                        "UPDATE inbox_item SET raw_text = ?2 WHERE id = ?1",
                        params![entity_id, s],
                    )?;
                }
            }
            "source" => {
                let s = req_str(val, field)?;
                let src = InboxSource::from_str(s)
                    .map_err(|e| OpLogError::InvalidValue(e.to_string()))?;
                if lww_should_apply(tx, entity_type, entity_id, field, op_hlc)? {
                    tx.execute(
                        "UPDATE inbox_item SET source = ?2 WHERE id = ?1",
                        params![entity_id, src.as_str()],
                    )?;
                }
            }
            "status" => {
                let s = req_str(val, field)?;
                let st = InboxStatus::from_str(s)
                    .map_err(|e| OpLogError::InvalidValue(e.to_string()))?;
                if lww_should_apply(tx, entity_type, entity_id, field, op_hlc)? {
                    tx.execute(
                        "UPDATE inbox_item SET status = ?2 WHERE id = ?1",
                        params![entity_id, st.as_str()],
                    )?;
                }
            }
            "processed_at" => {
                let ts = opt_i64(val, field)?;
                if lww_should_apply(tx, entity_type, entity_id, field, op_hlc)? {
                    tx.execute(
                        "UPDATE inbox_item SET processed_at = ?2 WHERE id = ?1",
                        params![entity_id, ts],
                    )?;
                }
            }
            other => {
                return Err(OpLogError::UnknownField {
                    entity_type: "inbox_item".to_string(),
                    field: other.to_string(),
                });
            }
        },
        EntityType::Project => match field {
            "name" => {
                let s = req_str(val, field)?;
                if lww_should_apply(tx, entity_type, entity_id, field, op_hlc)? {
                    tx.execute(
                        "UPDATE project SET name = ?2 WHERE id = ?1",
                        params![entity_id, s],
                    )?;
                }
            }
            "archived" => {
                let b = req_bool(val, field)?;
                if lww_should_apply(tx, entity_type, entity_id, field, op_hlc)? {
                    tx.execute(
                        "UPDATE project SET archived = ?2 WHERE id = ?1",
                        params![entity_id, i64::from(b)],
                    )?;
                }
            }
            other => {
                return Err(OpLogError::UnknownField {
                    entity_type: "project".to_string(),
                    field: other.to_string(),
                });
            }
        },
        EntityType::Task => apply_task_field(tx, entity_id, op_hlc, op_device_id, field, val)?,
        EntityType::Note => apply_note_field(tx, entity_id, op_hlc, field, val)?,
        EntityType::Transcript => match field {
            "text" => {
                let s = req_str(val, field)?;
                if lww_should_apply(tx, entity_type, entity_id, field, op_hlc)? {
                    tx.execute(
                        "UPDATE transcript SET text = ?2 WHERE id = ?1",
                        params![entity_id, s],
                    )?;
                }
            }
            "started_at" => {
                let ts = req_i64(val, field)?;
                if lww_should_apply(tx, entity_type, entity_id, field, op_hlc)? {
                    tx.execute(
                        "UPDATE transcript SET started_at = ?2 WHERE id = ?1",
                        params![entity_id, ts],
                    )?;
                }
            }
            "duration_sec" => {
                let dur = req_i64(val, field)?;
                if dur < 0 {
                    return Err(OpLogError::InvalidValue(
                        "duration_sec must be non-negative".to_string(),
                    ));
                }
                if lww_should_apply(tx, entity_type, entity_id, field, op_hlc)? {
                    tx.execute(
                        "UPDATE transcript SET duration_sec = ?2 WHERE id = ?1",
                        params![entity_id, dur],
                    )?;
                }
            }
            "consent_ack" => {
                let ack = req_bool(val, field)?;
                if lww_should_apply(tx, entity_type, entity_id, field, op_hlc)? {
                    tx.execute(
                        "UPDATE transcript SET consent_ack = ?2 WHERE id = ?1",
                        params![entity_id, i64::from(ack)],
                    )?;
                }
            }
            other => {
                return Err(OpLogError::UnknownField {
                    entity_type: "transcript".to_string(),
                    field: other.to_string(),
                });
            }
        },
        EntityType::UserProfile => {
            return Err(OpLogError::UnknownField {
                entity_type: "user_profile".to_string(),
                field: field.to_string(),
            });
        }
    }
    Ok(())
}

fn apply_task_field(
    tx: &Transaction<'_>,
    entity_id: &str,
    op_hlc: &str,
    op_device_id: &str,
    field: &str,
    val: &Value,
) -> Result<(), OpLogError> {
    let et = EntityType::Task;
    match field {
        "project_id" => {
            let id_opt = opt_uuid_str(val, field)?;
            if lww_should_apply(tx, et, entity_id, field, op_hlc)? {
                tx.execute(
                    "UPDATE task SET project_id = ?2 WHERE id = ?1",
                    params![entity_id, id_opt],
                )?;
            }
        }
        "title" => {
            let s = req_str(val, field)?;
            if lww_should_apply(tx, et, entity_id, field, op_hlc)? {
                tx.execute(
                    "UPDATE task SET title = ?2 WHERE id = ?1",
                    params![entity_id, s],
                )?;
            }
        }
        "category" => {
            let s = req_str(val, field)?;
            let cat =
                TaskCategory::from_str(s).map_err(|e| OpLogError::InvalidValue(e.to_string()))?;
            if lww_should_apply(tx, et, entity_id, field, op_hlc)? {
                tx.execute(
                    "UPDATE task SET category = ?2 WHERE id = ?1",
                    params![entity_id, cat.as_str()],
                )?;
            }
        }
        "parent_id" => {
            let id_opt = opt_uuid_str(val, field)?;
            if lww_should_apply(tx, et, entity_id, field, op_hlc)? {
                tx.execute(
                    "UPDATE task SET parent_id = ?2 WHERE id = ?1",
                    params![entity_id, id_opt],
                )?;
            }
        }
        "position" => {
            let pos = req_str(val, field)?;
            if pos.is_empty() {
                return Err(OpLogError::InvalidValue(
                    "task.position must not be empty".into(),
                ));
            }
            if lww_should_apply(tx, et, entity_id, field, op_hlc)? {
                tx.execute(
                    "UPDATE task SET position = ?2, position_device_id = ?3 WHERE id = ?1",
                    params![entity_id, pos, op_device_id],
                )?;
            }
        }
        "granularity" => {
            let g = opt_i64(val, field)?;
            if let Some(level) = g
                && !(1..=5).contains(&level)
            {
                return Err(OpLogError::InvalidValue(format!(
                    "granularity {level} out of 1..=5"
                )));
            }
            if lww_should_apply(tx, et, entity_id, field, op_hlc)? {
                tx.execute(
                    "UPDATE task SET granularity = ?2 WHERE id = ?1",
                    params![entity_id, g],
                )?;
            }
        }
        "status" => {
            let s = req_str(val, field)?;
            let st =
                TaskStatus::from_str(s).map_err(|e| OpLogError::InvalidValue(e.to_string()))?;
            if lww_should_apply(tx, et, entity_id, field, op_hlc)? {
                tx.execute(
                    "UPDATE task SET status = ?2 WHERE id = ?1",
                    params![entity_id, st.as_str()],
                )?;
            }
        }
        "due_at" => {
            let ts = opt_i64(val, field)?;
            if lww_should_apply(tx, et, entity_id, field, op_hlc)? {
                tx.execute(
                    "UPDATE task SET due_at = ?2 WHERE id = ?1",
                    params![entity_id, ts],
                )?;
            }
        }
        "due_precision" => {
            let prec = opt_str(val, field)?
                .map(|s| DuePrecision::from_str(s).map(|p| p.as_str()))
                .transpose()
                .map_err(|e| OpLogError::InvalidValue(e.to_string()))?;
            if lww_should_apply(tx, et, entity_id, field, op_hlc)? {
                tx.execute(
                    "UPDATE task SET due_precision = ?2 WHERE id = ?1",
                    params![entity_id, prec],
                )?;
            }
        }
        "urgency" => {
            let s = opt_str(val, field)?;
            if lww_should_apply(tx, et, entity_id, field, op_hlc)? {
                tx.execute(
                    "UPDATE task SET urgency = ?2 WHERE id = ?1",
                    params![entity_id, s],
                )?;
            }
        }
        "target_app_id" => {
            let s = opt_str(val, field)?;
            if lww_should_apply(tx, et, entity_id, field, op_hlc)? {
                tx.execute(
                    "UPDATE task SET target_app_id = ?2 WHERE id = ?1",
                    params![entity_id, s],
                )?;
            }
        }
        "est_p50_sec" | "est_p80_sec" | "est_p90_sec" => {
            let sec = opt_i64(val, field)?;
            if let Some(v) = sec
                && v < 0
            {
                return Err(OpLogError::InvalidValue(format!("{field} must be >= 0")));
            }
            if lww_should_apply(tx, et, entity_id, field, op_hlc)? {
                let sql = format!("UPDATE task SET {field} = ?2 WHERE id = ?1");
                tx.execute(&sql, params![entity_id, sec])?;
            }
        }
        "actual_sec" => {
            let sec = req_i64(val, field)?;
            if sec < 0 {
                return Err(OpLogError::InvalidValue("actual_sec must be >= 0".into()));
            }
            tx.execute(
                "UPDATE task SET actual_sec = MAX(actual_sec, ?2) WHERE id = ?1",
                params![entity_id, sec],
            )?;
            let _ = lww_should_apply(tx, et, entity_id, field, op_hlc)?;
        }
        "origin" => {
            let s = req_str(val, field)?;
            let orig = Origin::from_str(s).map_err(|e| OpLogError::InvalidValue(e.to_string()))?;
            if lww_should_apply(tx, et, entity_id, field, op_hlc)? {
                tx.execute(
                    "UPDATE task SET origin = ?2 WHERE id = ?1",
                    params![entity_id, orig.as_str()],
                )?;
            }
        }
        "origin_model" => {
            let s = opt_str(val, field)?;
            if lww_should_apply(tx, et, entity_id, field, op_hlc)? {
                tx.execute(
                    "UPDATE task SET origin_model = ?2 WHERE id = ?1",
                    params![entity_id, s],
                )?;
            }
        }
        "origin_prompt_ver" => {
            let s = opt_str(val, field)?;
            if lww_should_apply(tx, et, entity_id, field, op_hlc)? {
                tx.execute(
                    "UPDATE task SET origin_prompt_ver = ?2 WHERE id = ?1",
                    params![entity_id, s],
                )?;
            }
        }
        "accepted_at" => {
            if let Some(ts) = opt_i64(val, field)? {
                apply_first_write_accepted_at(tx, et, "task", entity_id, op_hlc, ts)?;
            }
        }
        other => {
            return Err(OpLogError::UnknownField {
                entity_type: "task".to_string(),
                field: other.to_string(),
            });
        }
    }
    Ok(())
}

fn apply_note_field(
    tx: &Transaction<'_>,
    entity_id: &str,
    op_hlc: &str,
    field: &str,
    val: &Value,
) -> Result<(), OpLogError> {
    let et = EntityType::Note;
    match field {
        "kind" => {
            let s = req_str(val, field)?;
            let kind =
                NoteKind::from_str(s).map_err(|e| OpLogError::InvalidValue(e.to_string()))?;
            if lww_should_apply(tx, et, entity_id, field, op_hlc)? {
                tx.execute(
                    "UPDATE note SET kind = ?2 WHERE id = ?1",
                    params![entity_id, kind.as_str()],
                )?;
            }
        }
        "text" => {
            let s = req_str(val, field)?;
            if lww_should_apply(tx, et, entity_id, field, op_hlc)? {
                tx.execute(
                    "UPDATE note SET text = ?2 WHERE id = ?1",
                    params![entity_id, s],
                )?;
            }
        }
        "starts_at" => {
            let ts = opt_i64(val, field)?;
            if lww_should_apply(tx, et, entity_id, field, op_hlc)? {
                tx.execute(
                    "UPDATE note SET starts_at = ?2 WHERE id = ?1",
                    params![entity_id, ts],
                )?;
            }
        }
        "answered" => {
            let ans = req_bool(val, field)?;
            tx.execute(
                "UPDATE note SET answered = CASE WHEN answered = 1 OR ?2 = 1 THEN 1 ELSE 0 END WHERE id = ?1",
                params![entity_id, i64::from(ans)],
            )?;
            let _ = lww_should_apply(tx, et, entity_id, field, op_hlc)?;
        }
        "origin" => {
            let s = req_str(val, field)?;
            let orig = Origin::from_str(s).map_err(|e| OpLogError::InvalidValue(e.to_string()))?;
            if lww_should_apply(tx, et, entity_id, field, op_hlc)? {
                tx.execute(
                    "UPDATE note SET origin = ?2 WHERE id = ?1",
                    params![entity_id, orig.as_str()],
                )?;
            }
        }
        "origin_model" => {
            let s = opt_str(val, field)?;
            if lww_should_apply(tx, et, entity_id, field, op_hlc)? {
                tx.execute(
                    "UPDATE note SET origin_model = ?2 WHERE id = ?1",
                    params![entity_id, s],
                )?;
            }
        }
        "origin_prompt_ver" => {
            let s = opt_str(val, field)?;
            if lww_should_apply(tx, et, entity_id, field, op_hlc)? {
                tx.execute(
                    "UPDATE note SET origin_prompt_ver = ?2 WHERE id = ?1",
                    params![entity_id, s],
                )?;
            }
        }
        "accepted_at" => {
            if let Some(ts) = opt_i64(val, field)? {
                apply_first_write_accepted_at(tx, et, "note", entity_id, op_hlc, ts)?;
            }
        }
        other => {
            return Err(OpLogError::UnknownField {
                entity_type: "note".to_string(),
                field: other.to_string(),
            });
        }
    }
    Ok(())
}

/// First-write-wins for `accepted_at`: earliest HLC wins so a sync never un-accepts work
/// and concurrent accept races converge deterministically regardless of delivery order.
fn apply_first_write_accepted_at(
    tx: &Transaction<'_>,
    entity_type: EntityType,
    table: &str,
    entity_id: &str,
    op_hlc: &str,
    accepted_at_ms: i64,
) -> Result<(), OpLogError> {
    let existing = get_field_clock(tx, entity_type, entity_id, "accepted_at")?;
    let wins = match existing {
        None => true,
        Some(prev) => op_hlc < prev.as_str(),
    };
    if wins {
        upsert_field_clock(tx, entity_type, entity_id, "accepted_at", op_hlc)?;
        let sql = format!("UPDATE {table} SET accepted_at = ?2 WHERE id = ?1");
        tx.execute(&sql, params![entity_id, accepted_at_ms])?;
    }
    Ok(())
}

fn req_str<'a>(val: &'a Value, field: &str) -> Result<&'a str, OpLogError> {
    val.as_str()
        .ok_or_else(|| OpLogError::InvalidValue(format!("field '{field}' must be a string")))
}

fn opt_str<'a>(val: &'a Value, field: &str) -> Result<Option<&'a str>, OpLogError> {
    if val.is_null() {
        return Ok(None);
    }
    req_str(val, field).map(Some)
}

fn opt_uuid_str(val: &Value, field: &str) -> Result<Option<String>, OpLogError> {
    match opt_str(val, field)? {
        None => Ok(None),
        Some(s) => {
            let u = Uuid::parse_str(s).map_err(|e| {
                OpLogError::InvalidValue(format!("field '{field}' invalid UUID: {e}"))
            })?;
            Ok(Some(u.to_string()))
        }
    }
}

fn req_i64(val: &Value, field: &str) -> Result<i64, OpLogError> {
    val.as_i64()
        .ok_or_else(|| OpLogError::InvalidValue(format!("field '{field}' must be an integer")))
}

fn opt_i64(val: &Value, field: &str) -> Result<Option<i64>, OpLogError> {
    if val.is_null() {
        return Ok(None);
    }
    req_i64(val, field).map(Some)
}

fn req_bool(val: &Value, field: &str) -> Result<bool, OpLogError> {
    if let Some(b) = val.as_bool() {
        return Ok(b);
    }
    if let Some(n) = val.as_i64()
        && (n == 0 || n == 1)
    {
        return Ok(n == 1);
    }
    Err(OpLogError::InvalidValue(format!(
        "field '{field}' must be a boolean"
    )))
}
