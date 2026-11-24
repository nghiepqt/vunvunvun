#![forbid(unsafe_code)]

use ef_core::TimeBufferPolicy;
use serde::{Deserialize, Serialize};

/// Embedding dimensionality for `multilingual-e5-small`.
pub const EMBEDDING_DIM: usize = 384;

/// Predicted duration range returned by Tier-0 M1 estimator.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct DurationRange {
    pub p50_sec: u32,
    pub p80_sec: u32,
    pub p90_sec: u32,
}

impl DurationRange {
    #[must_use]
    pub const fn selected_upper_sec(&self, policy: TimeBufferPolicy) -> u32 {
        match policy {
            TimeBufferPolicy::P50 => self.p50_sec,
            TimeBufferPolicy::P80 => self.p80_sec,
            TimeBufferPolicy::P90 => self.p90_sec,
        }
    }
}

/// Empirical Bayes weight `w = n / (n + k)` with `k = 15`.
#[must_use]
pub fn empirical_bayes_blend(population_sec: f64, personal_sec: f64, completions: u32) -> f64 {
    let n = f64::from(completions);
    let w = n / (n + 15.0);
    w * personal_sec + (1.0 - w) * population_sec
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empirical_bayes_respects_cold_start_and_personal_shift() {
        assert!((empirical_bayes_blend(600.0, 1200.0, 0) - 600.0).abs() < f64::EPSILON);
        assert!((empirical_bayes_blend(600.0, 1200.0, 15) - 900.0).abs() < f64::EPSILON);
    }
}
