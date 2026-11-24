#![forbid(unsafe_code)]

use std::path::PathBuf;

pub const MAX_RESTART_ATTEMPTS: u8 = 3;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SidecarConfig {
    pub executable_path: PathBuf,
    pub model_path: PathBuf,
    pub port: u16,
    pub memory_hard_limit_mb: u64,
}

impl SidecarConfig {
    /// Required CLI args for `llama-server` (`CONTEXT.md` / `PHASE-00` Spike W3).
    #[must_use]
    pub fn cli_args(&self) -> Vec<String> {
        vec![
            "--model".to_string(),
            self.model_path.display().to_string(),
            "--port".to_string(),
            self.port.to_string(),
            "--sleep-idle-seconds".to_string(),
            "-1".to_string(),
            "--slots".to_string(),
        ]
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SidecarState {
    Stopped,
    Starting,
    Ready,
    DisabledForSession,
}

#[derive(Debug)]
pub struct SidecarSupervisor {
    state: SidecarState,
    failures: u8,
}

impl Default for SidecarSupervisor {
    fn default() -> Self {
        Self::new()
    }
}

impl SidecarSupervisor {
    #[must_use]
    pub const fn new() -> Self {
        Self {
            state: SidecarState::Stopped,
            failures: 0,
        }
    }

    #[must_use]
    pub const fn state(&self) -> SidecarState {
        self.state
    }

    /// Records a crash; after 3 consecutive failures, disables Tier-1 local LLM for this session.
    pub fn record_crash(&mut self) -> SidecarState {
        self.failures = self.failures.saturating_add(1);
        if self.failures >= MAX_RESTART_ATTEMPTS {
            self.state = SidecarState::DisabledForSession;
        } else {
            self.state = SidecarState::Stopped;
        }
        self.state
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn disables_local_tier_after_three_crashes() {
        let mut sup = SidecarSupervisor::new();
        assert_eq!(sup.record_crash(), SidecarState::Stopped);
        assert_eq!(sup.record_crash(), SidecarState::Stopped);
        assert_eq!(sup.record_crash(), SidecarState::DisabledForSession);
    }
}
