use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DemandStyle {
    Invitational,
    Direct,
    Neutral,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LanguageMode {
    Natural,
    Literal,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Affect {
    Warm,
    Minimal,
    Playful,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StepVerbosity {
    Terse,
    Normal,
    Explanatory,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskCategory {
    Admin,
    DeepWork,
    Communication,
    Errand,
    Creative,
    General,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlanVisibility {
    NextOne,
    NextThree,
    All,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MotionLevel {
    Full,
    Reduced,
    None,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ColorIntensity {
    Standard,
    Muted,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Density {
    Compact,
    Standard,
    Spacious,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EnergyScale {
    Two,
    Three,
    Five,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TimeBufferPolicy {
    P50,
    P80,
    P90,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CloudAiMode {
    AskEachTime,
    Always,
    Never,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EfChallenge {
    TaskInitiation,
    WorkingMemory,
    TimeBlindness,
    TransitionCost,
    DemandAvoidance,
    SensoryOverload,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NudgeSettings {
    pub enabled: bool,
    pub daily_cap: u8,
    pub quiet_hours_enabled: bool,
}

impl Default for NudgeSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            daily_cap: 3,
            quiet_hours_enabled: true,
        }
    }
}

/// First-class user preference profile injected into prompt prefixes and UI rendering.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UserProfile {
    pub schema_version: u32,
    pub demand_style: DemandStyle,
    pub language_mode: LanguageMode,
    pub affect: Affect,
    pub step_verbosity: StepVerbosity,
    pub default_granularity: BTreeMap<TaskCategory, u8>,
    pub require_physical_first_step: bool,
    pub plan_visibility: PlanVisibility,
    pub motion: MotionLevel,
    pub color_intensity: ColorIntensity,
    pub sound_cues: bool,
    pub density: Density,
    /// Explicit self-declared preference (0..=100). Never learned or inferred.
    pub novelty_preference: u8,
    pub transition_warning_min: u16,
    pub nudge: NudgeSettings,
    pub energy_scale: EnergyScale,
    pub social_task_cost_multiplier: f64,
    pub time_buffer_policy: TimeBufferPolicy,
    pub show_elapsed_time: bool,
    /// Must remain false by default; countdowns are a demand signal.
    pub show_countdowns: bool,
    pub interests: Vec<String>,
    pub anchoring_enabled: bool,
    pub ef_challenges: Vec<EfChallenge>,
    pub cloud_ai: CloudAiMode,
    pub telemetry: bool,
    pub sync_enabled: bool,
}

impl Default for UserProfile {
    fn default() -> Self {
        Self {
            schema_version: 1,
            demand_style: DemandStyle::Invitational,
            language_mode: LanguageMode::Natural,
            affect: Affect::Warm,
            step_verbosity: StepVerbosity::Normal,
            default_granularity: BTreeMap::new(),
            require_physical_first_step: false,
            plan_visibility: PlanVisibility::NextThree,
            motion: MotionLevel::Reduced,
            color_intensity: ColorIntensity::Standard,
            sound_cues: false,
            density: Density::Standard,
            novelty_preference: 50,
            transition_warning_min: 10,
            nudge: NudgeSettings::default(),
            energy_scale: EnergyScale::Three,
            social_task_cost_multiplier: 1.0,
            time_buffer_policy: TimeBufferPolicy::P80,
            show_elapsed_time: false,
            show_countdowns: false,
            interests: Vec::new(),
            anchoring_enabled: false,
            ef_challenges: Vec::new(),
            cloud_ai: CloudAiMode::AskEachTime,
            telemetry: false,
            sync_enabled: false,
        }
    }
}

impl UserProfile {
    /// Computes the 6-bucket prompt-cache cluster identifier (ADR-003 / Phase 04).
    ///
    /// Folds `DemandStyle::Neutral` into `Direct` while keeping `LanguageMode::Literal`
    /// enforced by the deterministic validator.
    #[must_use]
    pub fn cluster_id(&self) -> String {
        let demand = match self.demand_style {
            DemandStyle::Invitational => "invitational",
            DemandStyle::Direct | DemandStyle::Neutral => "direct",
        };
        let lang = match self.language_mode {
            LanguageMode::Natural => "natural",
            LanguageMode::Literal => "literal",
        };
        let verbosity = match self.step_verbosity {
            StepVerbosity::Terse => "terse",
            StepVerbosity::Normal => "normal",
            StepVerbosity::Explanatory => "explanatory",
        };
        format!("{demand}-{lang}-{verbosity}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn safe_low_demand_defaults_hold() {
        let profile = UserProfile::default();
        assert!(!profile.show_countdowns);
        assert_eq!(profile.cloud_ai, CloudAiMode::AskEachTime);
        assert_eq!(profile.novelty_preference, 50);
        assert!(!profile.telemetry);
        assert_eq!(profile.cluster_id(), "invitational-natural-normal");
    }
}
