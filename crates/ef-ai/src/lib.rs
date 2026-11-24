#![forbid(unsafe_code)]

use ef_ai_transport::RouterDispatchToken;
use ef_core::{CloudAiMode, UserProfile};
use ef_protocol::{AiTier, FeatureId, StopReason};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RouteDecision {
    UseTier {
        tier: AiTier,
        output_cap: u32,
    },
    DeterministicFallback,
    ParkForConsent,
    SkipCall,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RouterContext {
    pub t1_available: bool,
    pub is_background_job: bool,
    pub spend_cap_breached: bool,
    pub input_tokens: u32,
    pub granularity: u8,
    pub has_retrieved_context: bool,
    pub user_requested_multi_day_plan: bool,
    pub validation_failures: u8,
}

/// Central tier & model router (`CONTEXT.md` Section 6, `PHASE-04`).
pub struct Router;

impl Router {
    #[must_use]
    pub fn dispatch_token(&self) -> RouterDispatchToken {
        RouterDispatchToken::issue_for_router()
    }

    #[must_use]
    pub fn decide(&self, feature: FeatureId, profile: &UserProfile, ctx: RouterContext) -> RouteDecision {
        // Two strikes -> deterministic fallback, never a third attempt.
        if ctx.validation_failures >= 2 {
            return RouteDecision::DeterministicFallback;
        }

        let (desired_tier, output_cap) = match feature {
            FeatureId::DeterministicTier0 => return RouteDecision::UseTier { tier: AiTier::T0, output_cap: 0 },
            FeatureId::BraindumpStructure => {
                let tier = if ctx.t1_available && ctx.input_tokens <= 800 && ctx.validation_failures == 0 {
                    AiTier::T1
                } else {
                    AiTier::T2
                };
                (tier, 400)
            }
            FeatureId::TaskDecompose => {
                let cap = if ctx.granularity >= 4 { 2000 } else { 500 };
                let tier = if ctx.user_requested_multi_day_plan {
                    AiTier::T3
                } else if ctx.granularity >= 4
                    || ctx.has_retrieved_context
                    || !ctx.t1_available
                    || ctx.validation_failures > 0
                {
                    AiTier::T2
                } else {
                    AiTier::T1
                };
                (tier, cap)
            }
            FeatureId::TaskResplit => {
                let tier = if ctx.t1_available && ctx.validation_failures == 0 {
                    AiTier::T1
                } else {
                    AiTier::T2
                };
                (tier, 250)
            }
            FeatureId::ObstaclePhrase => {
                let tier = if ctx.t1_available { AiTier::T1 } else { AiTier::T2 };
                (tier, 60)
            }
            FeatureId::ObstacleInterpret => {
                let tier = if ctx.t1_available && ctx.input_tokens <= 200 {
                    AiTier::T1
                } else {
                    AiTier::T2
                };
                (tier, 120)
            }
            FeatureId::TranscriptSummarize => (AiTier::T2, 800),
            FeatureId::ReviewNarrate => (AiTier::T3, 600),
            FeatureId::NudgeRephrase => {
                if ctx.t1_available && ctx.validation_failures == 0 {
                    return RouteDecision::UseTier {
                        tier: AiTier::T1,
                        output_cap: 40,
                    };
                }
                return RouteDecision::SkipCall;
            }
        };

        if matches!(desired_tier, AiTier::T2 | AiTier::T3) {
            if profile.cloud_ai == CloudAiMode::Never || ctx.spend_cap_breached {
                return RouteDecision::DeterministicFallback;
            }
            if profile.cloud_ai == CloudAiMode::AskEachTime && ctx.is_background_job {
                return RouteDecision::ParkForConsent;
            }
        }

        RouteDecision::UseTier {
            tier: desired_tier,
            output_cap,
        }
    }

    /// Truncation and refusal route directly to deterministic fallback without consuming a retry.
    #[must_use]
    pub const fn should_bypass_retry(stop_reason: StopReason) -> bool {
        matches!(stop_reason, StopReason::MaxTokens | StopReason::Refusal)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cloud_never_and_background_ask_each_time_rules_hold() {
        let router = Router;
        let mut profile = UserProfile::default();
        let ctx = RouterContext {
            t1_available: false,
            is_background_job: true,
            spend_cap_breached: false,
            input_tokens: 120,
            granularity: 3,
            has_retrieved_context: false,
            user_requested_multi_day_plan: false,
            validation_failures: 0,
        };

        assert_eq!(
            router.decide(FeatureId::BraindumpStructure, &profile, ctx),
            RouteDecision::ParkForConsent
        );

        profile.cloud_ai = CloudAiMode::Never;
        assert_eq!(
            router.decide(FeatureId::BraindumpStructure, &profile, ctx),
            RouteDecision::DeterministicFallback
        );
    }

    #[test]
    fn nudge_rephrase_never_escalates_to_cloud() {
        let router = Router;
        let mut profile = UserProfile::default();
        profile.cloud_ai = CloudAiMode::Always;
        let ctx = RouterContext {
            t1_available: false,
            is_background_job: false,
            spend_cap_breached: false,
            input_tokens: 20,
            granularity: 3,
            has_retrieved_context: false,
            user_requested_multi_day_plan: false,
            validation_failures: 0,
        };
        assert_eq!(
            router.decide(FeatureId::NudgeRephrase, &profile, ctx),
            RouteDecision::SkipCall
        );
    }

    #[test]
    fn stop_reason_truncation_bypasses_retry() {
        assert!(Router::should_bypass_retry(StopReason::MaxTokens));
        assert!(Router::should_bypass_retry(StopReason::Refusal));
        assert!(!Router::should_bypass_retry(StopReason::Stop));
    }
}
