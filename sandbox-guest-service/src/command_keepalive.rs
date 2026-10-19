//! Keep the fixed Go observation separate from the approved HTTP policy.
//! Both borrow the first field after the original total-header budget. Neither
//! allocates a timer or retains the original field. HTTP permits positive whole
//! seconds representable as signed nanoseconds; it never adopts Go's wrapping.

use axum::http::HeaderMap;
use std::time::Duration;

#[cfg(test)]
mod tests;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Selection {
    /// The exact positive interval accepted by the original Go constructor.
    Period(Duration),
    /// The signed nanosecond argument for which the original constructor panics.
    /// This is an observation, not an approved public failure or execution rule.
    UpstreamPanic { nanoseconds: i64 },
}

/// Select the first original field, including an explicitly empty first value.
/// Missing, invalid decimal and values outside signed 64-bit range fall back to
/// the fixed Go 90 seconds; later fields cannot repair or override the first.
#[must_use]
pub fn select(headers: &HeaderMap) -> Selection {
    select_first(first(headers))
}

fn first(headers: &HeaderMap) -> &[u8] {
    headers
        .get("keepalive-ping-interval")
        .map_or(b"", |value| value.as_bytes())
}
fn seconds(value: &[u8]) -> Option<i64> {
    std::str::from_utf8(value)
        .ok()
        .and_then(|text| text.parse::<i64>().ok())
}

/// Approved rejection of a parsed integer, without echoing untrusted input.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InvalidInterval;

/// HTTP Start/Connect policy: missing or failed i64 parsing uses 90 seconds.
/// Later fields never replace the first, including an empty first field.
/// # Errors
/// A successfully parsed integer outside `1..=9_223_372_036` seconds. The caller
/// must reject before execution, rather than using an upstream wrapped period.
pub fn bounded_period(headers: &HeaderMap) -> Result<Duration, InvalidInterval> {
    let seconds = seconds(first(headers)).unwrap_or(90);
    let nanoseconds = seconds
        .checked_mul(1_000_000_000)
        .filter(|value| *value > 0)
        .ok_or(InvalidInterval)?;
    Ok(Duration::from_nanos(nanoseconds.unsigned_abs()))
}

fn select_first(value: &[u8]) -> Selection {
    let Some(seconds) = seconds(value) else {
        return Selection::Period(Duration::from_secs(90));
    };
    // Go time.Duration is signed int64. Atoi overflow instead took the default
    // above; the later multiplication wraps, even for valid negative seconds.
    let nanoseconds = seconds.wrapping_mul(1_000_000_000);
    if nanoseconds > 0 {
        Selection::Period(Duration::from_nanos(nanoseconds.unsigned_abs()))
    } else {
        Selection::UpstreamPanic { nanoseconds }
    }
}
