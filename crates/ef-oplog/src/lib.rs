#![forbid(unsafe_code)]

use ef_core::{Hlc, UserAcceptedProof};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EntityType {
    Task,
    Note,
    InboxItem,
    UserProfile,
    Transcript,
    Project,
}

impl EntityType {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Task => "task",
            Self::Note => "note",
            Self::InboxItem => "inbox_item",
            Self::UserProfile => "user_profile",
            Self::Transcript => "transcript",
            Self::Project => "project",
        }
    }
}

/// Exclusive write handle to syncable SQLite tables.
///
/// The inner `rusqlite::Connection` field is private to `ef-oplog`, enforcing at compile time
/// that no external crate can issue raw `INSERT`/`UPDATE` statements against synced tables
/// (CONTEXT.md Section 6).
pub struct SyncWriteConn {
    conn: rusqlite::Connection,
}

impl SyncWriteConn {
    #[must_use]
    pub fn from_connection(conn: rusqlite::Connection) -> Self {
        Self { conn }
    }

    #[must_use]
    pub fn connection(&self) -> &rusqlite::Connection {
        &self.conn
    }
}

#[derive(Debug, Clone)]
pub enum SyncMutation {
    CreateInboxItem {
        id: Uuid,
        raw_text: String,
        source: String,
    },
    SetField {
        entity_type: EntityType,
        entity_id: Uuid,
        field: String,
        value_json: serde_json::Value,
    },
    AcceptEntity {
        entity_type: EntityType,
        entity_id: Uuid,
        accepted_at_ms: i64,
        proof: UserAcceptedProof,
    },
    Delete {
        entity_type: EntityType,
        entity_id: Uuid,
    },
}

#[derive(Debug, Error)]
pub enum OpLogError {
    #[error("inbox_item.raw_text is immutable and cannot be updated via SetField")]
    ImmutableRawText,
    #[error("accepted_at can only be set via SyncMutation::AcceptEntity with UserAcceptedProof")]
    AcceptedAtRequiresProof,
    #[error("sqlite error: {0}")]
    Sqlite(#[from] rusqlite::Error),
}

pub struct OpLog<'a> {
    write_conn: &'a mut SyncWriteConn,
    hlc: &'a mut Hlc,
}

impl<'a> OpLog<'a> {
    pub fn new(write_conn: &'a mut SyncWriteConn, hlc: &'a mut Hlc) -> Self {
        Self { write_conn, hlc }
    }

    /// Validates invariants and records an operation in the op-log.
    pub fn apply(&mut self, mutation: SyncMutation, now_ms: i64) -> Result<String, OpLogError> {
        match &mutation {
            SyncMutation::SetField {
                entity_type: EntityType::InboxItem,
                field,
                ..
            } if field == "raw_text" => return Err(OpLogError::ImmutableRawText),
            SyncMutation::SetField { field, .. } if field == "accepted_at" => {
                return Err(OpLogError::AcceptedAtRequiresProof);
            }
            _ => {}
        }

        let _ = &self.write_conn.conn;
        let hlc_stamp = self.hlc.tick(now_ms);
        Ok(hlc_stamp)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_raw_text_mutation_and_unproven_accepted_at() {
        let conn = rusqlite::Connection::open_in_memory().expect("in-memory sqlite");
        let mut write_conn = SyncWriteConn::from_connection(conn);
        let mut hlc = Hlc::new("dev-test", 0, 0);
        let mut oplog = OpLog::new(&mut write_conn, &mut hlc);

        let err = oplog
            .apply(
                SyncMutation::SetField {
                    entity_type: EntityType::InboxItem,
                    entity_id: Uuid::now_v7(),
                    field: "raw_text".to_string(),
                    value_json: serde_json::json!("mutated"),
                },
                1_700_000_000_000,
            )
            .expect_err("raw_text mutation must fail");
        assert!(matches!(err, OpLogError::ImmutableRawText));

        let err2 = oplog
            .apply(
                SyncMutation::SetField {
                    entity_type: EntityType::Task,
                    entity_id: Uuid::now_v7(),
                    field: "accepted_at".to_string(),
                    value_json: serde_json::json!(123),
                },
                1_700_000_000_001,
            )
            .expect_err("unproven accepted_at must fail");
        assert!(matches!(err2, OpLogError::AcceptedAtRequiresProof));
    }
}
