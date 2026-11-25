use std::collections::BTreeMap;
use std::str::FromStr;

use ef_core::{
    ClockDriftTooLargeError, DuePrecision, HlcParseError, InboxSource, NoteKind, Origin,
    TaskCategory, TaskStatus, UserAcceptedProof, UserProfile,
};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
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

impl FromStr for EntityType {
    type Err = OpLogError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "task" => Ok(Self::Task),
            "note" => Ok(Self::Note),
            "inbox_item" => Ok(Self::InboxItem),
            "user_profile" => Ok(Self::UserProfile),
            "transcript" => Ok(Self::Transcript),
            "project" => Ok(Self::Project),
            other => Err(OpLogError::InvalidValue(format!(
                "unknown entity_type '{other}'"
            ))),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OpKind {
    Create,
    Set,
    Delete,
}

impl OpKind {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Create => "create",
            Self::Set => "set",
            Self::Delete => "delete",
        }
    }
}

impl FromStr for OpKind {
    type Err = OpLogError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "create" => Ok(Self::Create),
            "set" => Ok(Self::Set),
            "delete" => Ok(Self::Delete),
            other => Err(OpLogError::InvalidValue(format!("unknown op '{other}'"))),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApplyOutcome {
    Applied,
    Duplicate,
}

/// Immutable operation entry stored in `op_log` and exchanged during sync.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OpRecord {
    pub op_id: Uuid,
    pub hlc: String,
    pub device_id: String,
    pub entity_type: EntityType,
    pub entity_id: Uuid,
    pub op: OpKind,
    pub field: Option<String>,
    pub value: Option<serde_json::Value>,
}

/// Typed local write request accepted by [`crate::OpLog::apply`].
///
/// Notice `CreateTask` and `CreateNote` intentionally have no `accepted_at` parameter;
/// `accepted_at` can ONLY be recorded via [`SyncMutation::AcceptEntity`] holding a
/// [`UserAcceptedProof`].
#[derive(Debug, Clone)]
pub enum SyncMutation {
    CreateInboxItem {
        id: Uuid,
        raw_text: String,
        source: InboxSource,
    },
    CreateProject {
        id: Uuid,
        name: String,
    },
    CreateTask {
        id: Uuid,
        project_id: Option<Uuid>,
        title: String,
        category: TaskCategory,
        parent_id: Option<Uuid>,
        position: String,
        granularity: Option<u8>,
        status: TaskStatus,
        due_at: Option<i64>,
        due_precision: Option<DuePrecision>,
        urgency: Option<String>,
        target_app_id: Option<String>,
        est_p50_sec: Option<i64>,
        est_p80_sec: Option<i64>,
        est_p90_sec: Option<i64>,
        origin: Origin,
        origin_model: Option<String>,
        origin_prompt_ver: Option<String>,
    },
    CreateNote {
        id: Uuid,
        kind: NoteKind,
        text: String,
        starts_at: Option<i64>,
        origin: Origin,
        origin_model: Option<String>,
        origin_prompt_ver: Option<String>,
    },
    CreateTranscript {
        id: Uuid,
        text: String,
        started_at: i64,
        duration_sec: i64,
        consent_ack: bool,
    },
    CreateUserProfile {
        id: Uuid,
        profile: UserProfile,
    },
    SetField {
        entity_type: EntityType,
        entity_id: Uuid,
        field: String,
        value_json: serde_json::Value,
    },
    SetUserProfileFields {
        id: Uuid,
        fields: BTreeMap<String, serde_json::Value>,
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
    #[error("AcceptEntity is only valid for task and note entities, got {0}")]
    UnsupportedAcceptEntity(String),
    #[error("unknown or non-mutable field '{field}' for entity '{entity_type}'")]
    UnknownField { entity_type: String, field: String },
    #[error("invalid op-log value: {0}")]
    InvalidValue(String),
    #[error("missing HLC state; call OpLog::init_hlc first")]
    MissingHlcState,
    #[error("clock drift check failed: {0}")]
    ClockDrift(#[from] ClockDriftTooLargeError),
    #[error("invalid HLC timestamp: {0}")]
    HlcParse(#[from] HlcParseError),
    #[error("JSON serialization error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("sqlite error: {0}")]
    Sqlite(#[from] rusqlite::Error),
}
