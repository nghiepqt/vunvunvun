#![forbid(unsafe_code)]

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AiTier {
    T0,
    T1,
    T2,
    T3,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FeatureId {
    BraindumpStructure,
    TaskDecompose,
    TaskResplit,
    ObstaclePhrase,
    ObstacleInterpret,
    TranscriptSummarize,
    ReviewNarrate,
    NudgeRephrase,
    DeterministicTier0,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StopReason {
    Stop,
    MaxTokens,
    Refusal,
    Other,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LlmUsageRecord {
    pub feature: FeatureId,
    pub tier: AiTier,
    pub model: String,
    pub prompt_ver: String,
    pub input_tokens: u32,
    pub cached_tokens: u32,
    pub output_tokens: u32,
    pub latency_ms: u64,
    pub estimated_cost_usd: f64,
    pub stop_reason: StopReason,
    pub truncated: bool,
    pub valid: bool,
    pub retried: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GatewayChatRequest {
    pub feature: FeatureId,
    pub tier: AiTier,
    pub cluster_id: String,
    pub prompt_ver: String,
    pub max_output_tokens: u32,
    pub system_prefix: String,
    pub user_suffix: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GatewayChatResponse {
    pub content: String,
    pub stop_reason: StopReason,
    pub model: String,
}
