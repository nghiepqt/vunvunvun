use thiserror::Error;

/// Error returned when parsing a serialized HLC string fails.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum HlcParseError {
    #[error("invalid HLC format: expected `{{ms:013}}-{{counter:010}}-{{device_id}}`")]
    InvalidFormat,
    #[error("invalid HLC millisecond timestamp")]
    InvalidTimestamp,
    #[error("invalid HLC counter")]
    InvalidCounter,
    #[error("HLC counter {0} exceeds MAX_COUNTER")]
    CounterOutOfRange(u32),
    #[error("HLC device_id must not be empty")]
    EmptyDeviceId,
}

/// Parsed components of a serialized Hybrid Logical Clock string.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedHlc {
    pub ms: i64,
    pub counter: u32,
    pub device_id: String,
}

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

    /// Parses `{ms:013}-{counter:010}-{device_id}` (supporting hyphens inside `device_id`).
    pub fn parse(raw: &str) -> Result<ParsedHlc, HlcParseError> {
        let mut parts = raw.splitn(3, '-');
        let ms_str = parts.next().ok_or(HlcParseError::InvalidFormat)?;
        let counter_str = parts.next().ok_or(HlcParseError::InvalidFormat)?;
        let device_id = parts.next().ok_or(HlcParseError::InvalidFormat)?;

        if ms_str.len() != 13 || !ms_str.bytes().all(|b| b.is_ascii_digit()) {
            return Err(HlcParseError::InvalidTimestamp);
        }
        if counter_str.len() != 10 || !counter_str.bytes().all(|b| b.is_ascii_digit()) {
            return Err(HlcParseError::InvalidCounter);
        }
        if device_id.is_empty() {
            return Err(HlcParseError::EmptyDeviceId);
        }

        let ms = ms_str
            .parse::<i64>()
            .map_err(|_| HlcParseError::InvalidTimestamp)?;
        let counter = counter_str
            .parse::<u32>()
            .map_err(|_| HlcParseError::InvalidCounter)?;
        if counter > Self::MAX_COUNTER {
            return Err(HlcParseError::CounterOutOfRange(counter));
        }

        Ok(ParsedHlc {
            ms,
            counter,
            device_id: device_id.to_string(),
        })
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

    #[test]
    fn parse_round_trips_uuid_device_id_and_rejects_malformed() {
        let hlc = Hlc::new(
            "0193a8f0-1234-7abc-8def-0123456789ab",
            1_700_000_000_123,
            42,
        );
        let formatted = hlc.format();
        let parsed = Hlc::parse(&formatted).expect("valid HLC");
        assert_eq!(parsed.ms, 1_700_000_000_123);
        assert_eq!(parsed.counter, 42);
        assert_eq!(parsed.device_id, "0193a8f0-1234-7abc-8def-0123456789ab");

        assert!(matches!(
            Hlc::parse("bad"),
            Err(HlcParseError::InvalidFormat)
        ));
        assert!(matches!(
            Hlc::parse("123-0000000000-dev"),
            Err(HlcParseError::InvalidTimestamp)
        ));
        assert!(matches!(
            Hlc::parse("1700000000000-000-dev"),
            Err(HlcParseError::InvalidCounter)
        ));
        assert!(matches!(
            Hlc::parse("1700000000000-0000000000-"),
            Err(HlcParseError::EmptyDeviceId)
        ));
    }
}
