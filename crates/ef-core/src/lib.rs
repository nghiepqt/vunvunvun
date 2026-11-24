#![forbid(unsafe_code)]

pub mod consent;
pub mod hlc;
pub mod profile;

pub use consent::UserAcceptedProof;
pub use hlc::{ClockDriftTooLargeError, Hlc};
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
