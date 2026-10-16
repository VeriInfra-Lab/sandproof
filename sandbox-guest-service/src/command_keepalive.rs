//! Fixed 64-bit Go envd keepalive-header selection, without a timer or policy.
//! The HTTP adapter must enforce its original total-header budget first. This
//! observation does not enable an interval, reject a request, or map an error.
//! Nonpositive wrapped intervals require an explicit service policy: the fixed
//! Go helper panics in `NewTicker`, which must not be silently called a default.

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
    select_first(
        headers
            .get("keepalive-ping-interval")
            .map_or(b"", |value| value.as_bytes()),
    )
}

fn select_first(value: &[u8]) -> Selection {
    let seconds = std::str::from_utf8(value)
        .ok()
        .and_then(|text| text.parse::<i64>().ok());
    let Some(seconds) = seconds else {
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
