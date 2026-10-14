use axum::http::HeaderMap;
use std::{fmt, time::Duration};
use tokio::time::Instant;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TimeoutReason {
    TooLong,
    Invalid,
}

/// The selected value is bounded by the complete request-header budget before
/// copying. Keep it for a future fixed error envelope; never log it in Debug.
#[derive(Clone, PartialEq, Eq)]
pub struct InvalidTimeout {
    reason: TimeoutReason,
    value: Vec<u8>,
}
impl InvalidTimeout {
    #[must_use]
    pub const fn reason(&self) -> TimeoutReason {
        self.reason
    }
    #[must_use]
    pub fn value(&self) -> &[u8] {
        &self.value
    }
}
impl fmt::Debug for InvalidTimeout {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("InvalidTimeout")
            .field("reason", &self.reason)
            .finish_non_exhaustive()
    }
}

// Fixed connect-go v1.18.1 SetTimeout selects only the first field. Empty is
// absent; +/- signs count toward ten bytes. Zero/negative means already expired,
// unlike the SDK API's zero (which omits the header) or command execution time.
pub(super) fn parse(value: &[u8]) -> Result<Option<i64>, TimeoutReason> {
    if value.is_empty() {
        return Ok(None);
    }
    if value.len() > 10 {
        return Err(TimeoutReason::TooLong);
    }
    let (negative, digits) = match value[0] {
        b'-' => (true, &value[1..]),
        b'+' => (false, &value[1..]),
        _ => (false, value),
    };
    if digits.is_empty() || !digits.iter().all(u8::is_ascii_digit) {
        return Err(TimeoutReason::Invalid);
    }
    let millis = digits
        .iter()
        .fold(0_i64, |n, byte| n * 10 + i64::from(byte - b'0'));
    Ok(Some(if negative { -millis } else { millis }))
}

pub(super) fn deadline(
    headers: &HeaderMap,
    now: Instant,
    original: Instant,
) -> Result<Instant, InvalidTimeout> {
    let value = headers
        .get("connect-timeout-ms")
        .map_or(b"".as_slice(), |value| value.as_bytes());
    let millis = parse(value).map_err(|reason| InvalidTimeout {
        reason,
        value: value.to_vec(),
    })?;
    Ok(match millis {
        None => original,
        Some(millis) if millis <= 0 => now.min(original),
        Some(millis) => {
            // Compare before adding: a large valid remote duration must not
            // overflow a clock when the local admission ends much earlier.
            let duration = Duration::from_millis(millis.unsigned_abs());
            if duration >= original.saturating_duration_since(now) {
                original
            } else {
                now + duration
            }
        }
    })
}
