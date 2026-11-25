#![forbid(unsafe_code)]

pub mod consent;
pub mod domain;
pub mod hlc;
pub mod position;
pub mod profile;

pub use consent::UserAcceptedProof;
pub use domain::{
    DuePrecision, InboxItem, InboxSource, InboxStatus, Note, NoteKind, Origin,
    ParseDomainEnumError, Project, SyncMetadata, Task, TaskStatus, Transcript, UserProfileRecord,
};
pub use hlc::{ClockDriftTooLargeError, Hlc, HlcParseError, ParsedHlc};
pub use position::{
    PositionError, compare_position, position_after, position_before, position_between,
    position_initial,
};
pub use profile::{
    Affect, CloudAiMode, ColorIntensity, DemandStyle, Density, EfChallenge, EnergyScale,
    LanguageMode, MotionLevel, NudgeSettings, PlanVisibility, StepVerbosity, TaskCategory,
    TimeBufferPolicy, UserProfile,
};

/// Generates a time-sortable client identifier (`UUIDv7`) for syncable entities.
#[must_use]
pub fn new_entity_id() -> uuid::Uuid {
    uuid::Uuid::now_v7()
}
