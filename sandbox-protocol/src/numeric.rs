//! Pure unit conversions and bounded resource arithmetic used by both services.
//! These helpers do not choose admission errors, validate clocks or authorize
//! resource use. Their callers retain their original ordered policy checks.

/// Preserve the exact millisecond value, or reject a U64 multiplication overflow.
/// Zero remains zero; rejecting a zero lifetime is a caller policy.
#[must_use]
pub fn seconds_to_milliseconds(seconds: u64) -> Option<u64> {
    seconds.checked_mul(1000)
}

/// The original public disk-size projection: round upward in units of 2^20 bytes.
/// Reject values whose rounded size cannot be represented by the U32 field.
#[must_use]
#[expect(
    clippy::cast_possible_truncation,
    reason = "The preceding explicit U32 bound establishes that the cast preserves the value"
)]
pub fn bytes_to_mebibytes(bytes: u64) -> Option<u32> {
    let whole = bytes / (1024 * 1024);
    let extra = match bytes % (1024 * 1024) {
        0 => 0,
        _ => 1,
    };
    // whole is at most 2^44 - 1, so this addition cannot overflow U64.
    let units = whole + extra;
    if units > u64::from(u32::MAX) {
        None
    } else {
        Some(units as u32)
    }
}

/// Bound a reported resource component by its logical and observed availability.
/// Already-used capacity at or above the limit yields zero, never a wrapped value.
/// `observed` remains an input value, not a certificate of physical availability.
#[must_use]
pub fn available_resource(limit: u64, used: u64, observed: u64) -> u64 {
    let remaining = limit.saturating_sub(used);
    if remaining < observed {
        remaining
    } else {
        observed
    }
}
