use std::str::FromStr;

use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

use crate::{TaskCategory, UserProfile};

#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("invalid enum value '{value}' for {kind}")]
pub struct ParseDomainEnumError {
    pub kind: &'static str,
    pub value: String,
}

/// Common sync metadata carried by every syncable row (`PHASE-01-data-layer.md`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SyncMetadata {
    pub updated_at: i64,
    pub deleted_at: Option<i64>,
    pub device_id: String,
    pub hlc: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InboxSource {
    Hotkey,
    Voice,
    Image,
    Mobile,
    Import,
}

impl InboxSource {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Hotkey => "hotkey",
            Self::Voice => "voice",
            Self::Image => "image",
            Self::Mobile => "mobile",
            Self::Import => "import",
        }
    }
}

impl FromStr for InboxSource {
    type Err = ParseDomainEnumError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "hotkey" => Ok(Self::Hotkey),
            "voice" => Ok(Self::Voice),
            "image" => Ok(Self::Image),
            "mobile" => Ok(Self::Mobile),
            "import" => Ok(Self::Import),
            _ => Err(ParseDomainEnumError {
                kind: "InboxSource",
                value: s.to_string(),
            }),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InboxStatus {
    Pending,
    PendingConsent,
    Structured,
    Discarded,
}

impl InboxStatus {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::PendingConsent => "pending_consent",
            Self::Structured => "structured",
            Self::Discarded => "discarded",
        }
    }
}

impl FromStr for InboxStatus {
    type Err = ParseDomainEnumError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "pending" => Ok(Self::Pending),
            "pending_consent" => Ok(Self::PendingConsent),
            "structured" => Ok(Self::Structured),
            "discarded" => Ok(Self::Discarded),
            _ => Err(ParseDomainEnumError {
                kind: "InboxStatus",
                value: s.to_string(),
            }),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InboxItem {
    pub id: Uuid,
    pub raw_text: String,
    pub source: InboxSource,
    pub status: InboxStatus,
    pub created_at: i64,
    pub processed_at: Option<i64>,
    pub sync: SyncMetadata,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskStatus {
    Open,
    InProgress,
    Done,
    Parked,
}

impl TaskStatus {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Open => "open",
            Self::InProgress => "in_progress",
            Self::Done => "done",
            Self::Parked => "parked",
        }
    }
}

impl FromStr for TaskStatus {
    type Err = ParseDomainEnumError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "open" => Ok(Self::Open),
            "in_progress" => Ok(Self::InProgress),
            "done" => Ok(Self::Done),
            "parked" => Ok(Self::Parked),
            _ => Err(ParseDomainEnumError {
                kind: "TaskStatus",
                value: s.to_string(),
            }),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DuePrecision {
    Day,
    Hour,
    Minute,
}

impl DuePrecision {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Day => "day",
            Self::Hour => "hour",
            Self::Minute => "minute",
        }
    }
}

impl FromStr for DuePrecision {
    type Err = ParseDomainEnumError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "day" => Ok(Self::Day),
            "hour" => Ok(Self::Hour),
            "minute" => Ok(Self::Minute),
            _ => Err(ParseDomainEnumError {
                kind: "DuePrecision",
                value: s.to_string(),
            }),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Origin {
    User,
    T1,
    T2,
    T3,
}

impl Origin {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::User => "user",
            Self::T1 => "t1",
            Self::T2 => "t2",
            Self::T3 => "t3",
        }
    }
}

impl FromStr for Origin {
    type Err = ParseDomainEnumError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "user" => Ok(Self::User),
            "t1" => Ok(Self::T1),
            "t2" => Ok(Self::T2),
            "t3" => Ok(Self::T3),
            _ => Err(ParseDomainEnumError {
                kind: "Origin",
                value: s.to_string(),
            }),
        }
    }
}

impl TaskCategory {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Admin => "admin",
            Self::DeepWork => "deep_work",
            Self::Communication => "communication",
            Self::Errand => "errand",
            Self::Creative => "creative",
            Self::General => "general",
        }
    }
}

impl FromStr for TaskCategory {
    type Err = ParseDomainEnumError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "admin" => Ok(Self::Admin),
            "deep_work" => Ok(Self::DeepWork),
            "communication" => Ok(Self::Communication),
            "errand" => Ok(Self::Errand),
            "creative" => Ok(Self::Creative),
            "general" => Ok(Self::General),
            _ => Err(ParseDomainEnumError {
                kind: "TaskCategory",
                value: s.to_string(),
            }),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Task {
    pub id: Uuid,
    pub project_id: Option<Uuid>,
    pub title: String,
    pub category: TaskCategory,
    pub parent_id: Option<Uuid>,
    pub position: String,
    pub position_device_id: String,
    pub granularity: Option<u8>,
    pub status: TaskStatus,
    pub due_at: Option<i64>,
    pub due_precision: Option<DuePrecision>,
    pub urgency: Option<String>,
    pub target_app_id: Option<String>,
    pub est_p50_sec: Option<i64>,
    pub est_p80_sec: Option<i64>,
    pub est_p90_sec: Option<i64>,
    pub actual_sec: i64,
    pub origin: Origin,
    pub origin_model: Option<String>,
    pub origin_prompt_ver: Option<String>,
    pub accepted_at: Option<i64>,
    pub created_at: i64,
    pub sync: SyncMetadata,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NoteKind {
    Note,
    Event,
    Question,
    Decision,
}

impl NoteKind {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Note => "note",
            Self::Event => "event",
            Self::Question => "question",
            Self::Decision => "decision",
        }
    }
}

impl FromStr for NoteKind {
    type Err = ParseDomainEnumError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "note" => Ok(Self::Note),
            "event" => Ok(Self::Event),
            "question" => Ok(Self::Question),
            "decision" => Ok(Self::Decision),
            _ => Err(ParseDomainEnumError {
                kind: "NoteKind",
                value: s.to_string(),
            }),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Note {
    pub id: Uuid,
    pub kind: NoteKind,
    pub text: String,
    pub starts_at: Option<i64>,
    pub answered: bool,
    pub origin: Origin,
    pub origin_model: Option<String>,
    pub origin_prompt_ver: Option<String>,
    pub accepted_at: Option<i64>,
    pub created_at: i64,
    pub sync: SyncMetadata,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Project {
    pub id: Uuid,
    pub name: String,
    pub archived: bool,
    pub created_at: i64,
    pub sync: SyncMetadata,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Transcript {
    pub id: Uuid,
    pub text: String,
    pub started_at: i64,
    pub duration_sec: i64,
    pub consent_ack: bool,
    pub created_at: i64,
    pub sync: SyncMetadata,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UserProfileRecord {
    pub id: Uuid,
    pub profile: UserProfile,
    pub profile_hash: String,
    pub cluster_id: String,
    pub sync: SyncMetadata,
}
