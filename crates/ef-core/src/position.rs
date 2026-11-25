use std::cmp::Ordering;

use thiserror::Error;

const ALPHABET: &[u8; 62] = b"0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz";
const BASE: usize = 62;

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum PositionError {
    #[error("position string must not be empty")]
    EmptyPosition,
    #[error("position string contains invalid base-62 character '{0}'")]
    InvalidCharacter(char),
    #[error("cannot compute midpoint: '{before}' is not strictly less than '{after}'")]
    InvalidOrder { before: String, after: String },
    #[error("cannot compute position before minimal value '{0}'")]
    BelowMinimum(String),
}

fn char_to_index(byte: u8) -> Result<usize, PositionError> {
    match byte {
        b'0'..=b'9' => Ok((byte - b'0') as usize),
        b'A'..=b'Z' => Ok((byte - b'A') as usize + 10),
        b'a'..=b'z' => Ok((byte - b'a') as usize + 36),
        other => Err(PositionError::InvalidCharacter(char::from(other))),
    }
}

fn validate_position(pos: &str) -> Result<(), PositionError> {
    if pos.is_empty() {
        return Err(PositionError::EmptyPosition);
    }
    for byte in pos.bytes() {
        let _ = char_to_index(byte)?;
    }
    Ok(())
}

/// Returns the default initial base-62 fractional position (`"V"`).
#[must_use]
pub fn position_initial() -> String {
    "V".to_string()
}

/// Computes a valid base-62 fractional position strictly greater than `prev`.
pub fn position_after(prev: &str) -> Result<String, PositionError> {
    position_between(Some(prev), None)
}

/// Computes a valid base-62 fractional position strictly less than `next`.
pub fn position_before(next: &str) -> Result<String, PositionError> {
    position_between(None, Some(next))
}

/// Computes a variable-length base-62 fractional index strictly between `before` and `after`.
///
/// The alphabet `0-9A-Za-z` matches ASCII/SQLite `BINARY` sort order.
pub fn position_between(
    before: Option<&str>,
    after: Option<&str>,
) -> Result<String, PositionError> {
    match (before, after) {
        (None, None) => Ok(position_initial()),
        (Some(a), None) => {
            validate_position(a)?;
            let a_bytes = a.as_bytes();
            let mut out = Vec::with_capacity(a_bytes.len() + 1);
            let mut i = 0;
            loop {
                let da = if i < a_bytes.len() {
                    char_to_index(a_bytes[i])?
                } else {
                    0
                };
                if da + 1 < BASE {
                    let mid = da + (BASE - da) / 2;
                    out.push(ALPHABET[mid]);
                    break;
                }
                out.push(ALPHABET[da]);
                i += 1;
            }
            Ok(String::from_utf8(out).unwrap_or_else(|_| position_initial()))
        }
        (None, Some(b)) => {
            validate_position(b)?;
            if b.bytes().all(|byte| byte == b'0') {
                return Err(PositionError::BelowMinimum(b.to_string()));
            }
            compute_between_raw(b"", b.as_bytes())
        }
        (Some(a), Some(b)) => {
            validate_position(a)?;
            validate_position(b)?;
            if a >= b {
                return Err(PositionError::InvalidOrder {
                    before: a.to_string(),
                    after: b.to_string(),
                });
            }
            compute_between_raw(a.as_bytes(), b.as_bytes())
        }
    }
}

fn compute_between_raw(a: &[u8], b: &[u8]) -> Result<String, PositionError> {
    let mut out = Vec::with_capacity(a.len().max(b.len()) + 2);
    let mut i = 0;
    loop {
        let da = if i < a.len() { char_to_index(a[i])? } else { 0 };
        let db = if i < b.len() { char_to_index(b[i])? } else { 0 };

        if da == db {
            out.push(ALPHABET[da]);
            i += 1;
            continue;
        }

        if db - da > 1 {
            let mid = da + (db - da) / 2;
            out.push(ALPHABET[mid]);
            break;
        }

        // `db - da == 1`: fix `ALPHABET[da]` at index `i` (guaranteeing `< b`)
        // and extend past `a` until we find a digit `< BASE - 1`.
        out.push(ALPHABET[da]);
        let mut j = i + 1;
        loop {
            let da_next = if j < a.len() { char_to_index(a[j])? } else { 0 };
            if da_next + 1 < BASE {
                let mid = da_next + (BASE - da_next) / 2;
                out.push(ALPHABET[mid]);
                return Ok(String::from_utf8(out).unwrap_or_else(|_| position_initial()));
            }
            out.push(ALPHABET[da_next]);
            j += 1;
        }
    }
    Ok(String::from_utf8(out).unwrap_or_else(|_| position_initial()))
}

/// Deterministic total order for sibling items: `(position, position_device_id, id)`.
#[must_use]
pub fn compare_position(
    pos_a: &str,
    dev_a: &str,
    id_a: &str,
    pos_b: &str,
    dev_b: &str,
    id_b: &str,
) -> Ordering {
    pos_a
        .cmp(pos_b)
        .then_with(|| dev_a.cmp(dev_b))
        .then_with(|| id_a.cmp(id_b))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sequential_appends_and_prepends_stay_strictly_ordered() {
        let mut items = vec![position_initial()];
        for _ in 0..100 {
            let last = items.last().expect("non-empty");
            let next = position_after(last).expect("position_after");
            assert!(next.as_str() > last.as_str());
            items.push(next);
        }
        for _ in 0..100 {
            let first = items.first().expect("non-empty");
            let prev = position_before(first).expect("position_before");
            assert!(prev.as_str() < first.as_str());
            items.insert(0, prev);
        }
        assert!(items.windows(2).all(|w| w[0] < w[1]));
    }

    #[test]
    fn repeated_midpoint_bisection_never_collides() {
        let mut left = "A".to_string();
        let right = "B".to_string();
        for _ in 0..100 {
            let mid = position_between(Some(&left), Some(&right)).expect("midpoint");
            assert!(
                left < mid && mid < right,
                "failed: {left} < {mid} < {right}"
            );
            left = mid;
        }

        let mut prev_right = "0z".to_string();
        let bound = "1".to_string();
        for _ in 0..20 {
            let mid = position_between(Some(&prev_right), Some(&bound)).expect("midpoint near z");
            assert!(prev_right < mid && mid < bound);
            prev_right = mid;
        }
    }

    #[test]
    fn tie_breaks_deterministically_by_device_id_then_entity_id() {
        assert_eq!(
            compare_position("V", "dev-a", "id-2", "V", "dev-b", "id-1"),
            Ordering::Less
        );
        assert_eq!(
            compare_position("V", "dev-a", "id-1", "V", "dev-a", "id-2"),
            Ordering::Less
        );
    }
}
