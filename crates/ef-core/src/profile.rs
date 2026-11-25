use std::collections::BTreeMap;
use std::fmt::Write as _;

use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};

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

    /// Computes a deterministic SHA-256 hex digest over prompt-affecting preferences (ADR-003).
    ///
    /// UI-only settings (such as `density`, `color_intensity`, `motion`) do not alter
    /// `profile_hash`, preserving prompt cache hits across cosmetic UI tweaks.
    #[must_use]
    pub fn profile_hash(&self) -> String {
        #[derive(Serialize)]
        struct PromptProfileSlice<'a> {
            schema_version: u32,
            demand_style: DemandStyle,
            language_mode: LanguageMode,
            affect: Affect,
            step_verbosity: StepVerbosity,
            default_granularity: &'a BTreeMap<TaskCategory, u8>,
            require_physical_first_step: bool,
            novelty_preference: u8,
            anchoring_enabled: bool,
            ef_challenges: &'a [EfChallenge],
            interests: &'a [String],
        }

        let slice = PromptProfileSlice {
            schema_version: self.schema_version,
            demand_style: self.demand_style,
            language_mode: self.language_mode,
            affect: self.affect,
            step_verbosity: self.step_verbosity,
            default_granularity: &self.default_granularity,
            require_physical_first_step: self.require_physical_first_step,
            novelty_preference: self.novelty_preference,
            anchoring_enabled: self.anchoring_enabled,
            ef_challenges: &self.ef_challenges,
            interests: &self.interests,
        };

        let bytes = serde_json::to_vec(&slice).unwrap_or_default();
        let digest = Sha256::digest(&bytes);
        let mut hex = String::with_capacity(64);
        for byte in digest {
            let _ = write!(hex, "{byte:02x}");
        }
        hex
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

    #[test]
    fn profile_hash_ignores_ui_only_changes_and_tracks_prompt_preferences() {
        let base = UserProfile::default();
        let mut ui_only = base.clone();
        ui_only.density = Density::Compact;
        ui_only.color_intensity = ColorIntensity::Muted;
        assert_eq!(base.profile_hash(), ui_only.profile_hash());

        let mut prompt_changed = base.clone();
        prompt_changed.demand_style = DemandStyle::Direct;
        assert_ne!(base.profile_hash(), prompt_changed.profile_hash());
        assert_eq!(base.profile_hash().len(), 64);
    }
}
