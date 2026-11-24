#![forbid(unsafe_code)]

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum CandidateKind {
    Task,
    Note,
    Event,
    Question,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum BlockerRisk {
    Low,
    Medium,
    High,
}

/// Output contract for F2 structuring.
///
/// Invariant (CONTEXT.md Section 6): never emit resolved dates, times, durations, deadlines,
/// or numeric priorities. Copy the user's phrase verbatim into `temporal_phrase`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct StructureCandidate {
    pub kind: CandidateKind,
    pub text: String,
    pub confidence: f32,
    pub cited_fragment_ids: Vec<String>,
    pub temporal_phrase: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct StructureOutput {
    pub candidates: Vec<StructureCandidate>,
    pub clarifications_needed: Vec<String>,
    pub confidence: f32,
}

/// Output contract for F3 task decomposition.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct DecomposeStep {
    pub text: String,
    pub is_physical_action: bool,
    pub cited_fragment_ids: Vec<String>,
    pub blocker_risk: BlockerRisk,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct DecomposeOutput {
    pub steps: Vec<DecomposeStep>,
    pub clarifications_needed: Vec<String>,
    pub confidence: f32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PromptVersion {
    pub id: &'static str,
    pub version: &'static str,
}

pub const STRUCTURE_PROMPT_V1: PromptVersion = PromptVersion {
    id: "braindump.structure",
    version: "1.0.0",
};

pub const DECOMPOSE_PROMPT_V1: PromptVersion = PromptVersion {
    id: "task.decompose",
    version: "1.0.0",
};

#[cfg(test)]
mod tests {
    use super::*;
    use schemars::schema::{RootSchema, Schema};

    const FORBIDDEN_FIELDS: &[&str] = &[
        "date",
        "time",
        "duration",
        "deadline",
        "due_at",
        "starts_at",
        "priority",
        "numeric_estimate",
        "est_p50_sec",
        "est_p80_sec",
        "est_p90_sec",
    ];

    fn check_schema_object(schema: &Schema, path: &str) {
        if let Schema::Object(obj) = schema
            && let Some(object) = &obj.object
        {
            for (name, prop_schema) in &object.properties {
                let lower = name.to_ascii_lowercase();
                assert!(
                    !FORBIDDEN_FIELDS.contains(&lower.as_str()),
                    "forbidden temporal/numeric field `{name}` found at `{path}`"
                );
                check_schema_object(prop_schema, &format!("{path}.{name}"));
            }
        }
    }

    fn check_root(root: &RootSchema, label: &str) {
        check_schema_object(&Schema::Object(root.schema.clone()), label);
        for (def_name, def_schema) in &root.definitions {
            check_schema_object(def_schema, &format!("{label}::{def_name}"));
        }
    }

    #[test]
    fn output_schemas_never_emit_temporal_or_numeric_estimates() {
        check_root(&schemars::schema_for!(StructureOutput), "StructureOutput");
        check_root(&schemars::schema_for!(DecomposeOutput), "DecomposeOutput");
    }
}
