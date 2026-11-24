use thiserror::Error;

/// Error returned when a remote HLC timestamp exceeds the allowed clock drift bound.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("remote clock drift too large: remote_ms={remote_ms}, now_ms={now_ms}")]
pub struct ClockDriftTooLargeError {
    pub remote_ms: i64,
    pub now_ms: i64,
}

/// Hybrid Logical Clock producing lexicographically sortable `{ms:013}-{counter:010}-{device_id}` values.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hlc {
    last_ms: i64,
    counter: u32,
    device_id: String,
}

impl Hlc {
    /// Mandatory bounded drift check (60 seconds) so a skewed device clock cannot poison peers.
    pub const MAX_DRIFT_MS: i64 = 60_000;
    /// Must match the 10-digit padding width `{:010}` so counter overflow never breaks lexicographic order.
    pub const MAX_COUNTER: u32 = 999_999_999;

    #[must_use]
    pub fn new(device_id: impl Into<String>, last_ms: i64, counter: u32) -> Self {
        Self {
            last_ms: last_ms.max(0),
            counter: counter.min(Self::MAX_COUNTER),
            device_id: device_id.into(),
        }
    }

    #[must_use]
    pub fn state(&self) -> (i64, u32) {
        (self.last_ms, self.counter)
    }

    #[must_use]
    pub fn device_id(&self) -> &str {
        &self.device_id
    }

    /// Advances the clock for a local write. Callers must persist `state()` in `hlc_state`.
    pub fn tick(&mut self, now_ms: i64) -> String {
        let clamped_now = now_ms.max(0);
        if clamped_now > self.last_ms {
            self.last_ms = clamped_now;
            self.counter = 0;
        } else {
            self.counter += 1;
        }
        if self.counter >= Self::MAX_COUNTER {
            self.last_ms += 1;
            self.counter = 0;
        }
        self.format()
    }

    /// Observes a remote operation timestamp before applying it.
    pub fn observe(
        &mut self,
        remote_ms: i64,
        remote_counter: u32,
        now_ms: i64,
    ) -> Result<(), ClockDriftTooLargeError> {
        if remote_ms > now_ms + Self::MAX_DRIFT_MS {
            return Err(ClockDriftTooLargeError { remote_ms, now_ms });
        }

        let max_ms = now_ms.max(self.last_ms).max(remote_ms).max(0);
        self.counter = if max_ms == self.last_ms && max_ms == remote_ms {
            self.counter.max(remote_counter).saturating_add(1)
        } else if max_ms == self.last_ms {
            self.counter.saturating_add(1)
        } else if max_ms == remote_ms {
            remote_counter.saturating_add(1)
        } else {
            0
        };
        if self.counter >= Self::MAX_COUNTER {
            self.last_ms = max_ms + 1;
            self.counter = 0;
        } else {
            self.last_ms = max_ms;
        }
        Ok(())
    }

    #[must_use]
    pub fn format(&self) -> String {
        format!(
            "{:013}-{:010}-{}",
            self.last_ms, self.counter, self.device_id
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tick_stays_monotonic_across_backwards_clock_jump() {
        let mut hlc = Hlc::new("dev-a", 1_700_000_000_000, 0);
        let first = hlc.tick(1_700_000_000_010);
        let second = hlc.tick(1_700_000_000_005);
        assert!(second > first, "expected {second} > {first}");
    }

    #[test]
    fn observe_rejects_excessive_drift() {
        let mut hlc = Hlc::new("dev-a", 1_000_000, 0);
        let err = hlc
            .observe(1_000_000 + Hlc::MAX_DRIFT_MS + 1, 0, 1_000_000)
            .expect_err("drift over 60s must fail");
        assert_eq!(err.now_ms, 1_000_000);
    }

    #[test]
    fn counter_rollover_maintains_lexicographic_order() {
        let mut hlc = Hlc::new("dev-a", 1_700_000_000_000, Hlc::MAX_COUNTER - 1);
        let before = hlc.format();
        let after = hlc.tick(1_700_000_000_000);
        assert!(after > before, "expected {after} > {before}");
        assert_eq!(hlc.state(), (1_700_000_000_001, 0));
    }
}
