//! Canonical generated identities, distinct from opaque user-supplied IDs.

/// Parse one generated number in the original prefix and durable frontier.
/// This pure check does not authorize retirement, reuse, or execution.
#[must_use]
pub fn parse_allocated_number(value: &str, prefix: &str, highwater: u64) -> Option<u64> {
    // The exact prefix is checked before parsing. split_at_checked avoids the
    // fixed translator's unsupported Pattern type without slicing invalid UTF-8.
    let (actual_prefix, suffix) = value.split_at_checked(prefix.len())?;
    if actual_prefix != prefix {
        return None;
    }
    let value = suffix.parse::<u64>().ok()?;
    (value > 0 && value <= highwater && value.to_string() == suffix).then_some(value)
}
