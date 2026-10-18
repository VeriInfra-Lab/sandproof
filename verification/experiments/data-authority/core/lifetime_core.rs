//! Validation of raw lifetime fields and ordered live-authority checks.
#[path = "lifetime_bytes.rs"]
mod byte_core;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LifetimePhase {
    Starting,
    Running,
    Pausing,
    Paused,
    Revoking,
    Destroyed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LifetimeReject {
    InvalidClock,
    NotRunning,
    Unauthorized,
    MissingLifetime,
    Expired,
    CorruptLifetime,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Lifetime {
    /// `None` before successful startup is published.
    pub started_at_ms: Option<i64>,
    pub deadline_ms: i64,
    pub max_deadline_ms: i64,
    /// Immutable admission-time watchdog; never used to expire Running.
    pub startup_deadline_ms: i64,
    pub disk_bytes: u64,
}

#[derive(Clone, Copy)]
pub struct RawLifetimeRow<'a> {
    pub owner: &'a [u8],
    pub deadline_ms: i64,
    pub bytes: i64,
    pub started_at_ms: Option<i64>,
    pub max_deadline_ms: i64,
    pub startup_deadline_ms: i64,
}

#[must_use]
pub fn started_within_deadline(started: Option<i64>, deadline: i64) -> bool {
    if let Some(time) = started {
        if time < 0 {
            return false;
        }
        if time > deadline {
            return false;
        }
    }
    true
}

/// # Errors
/// Rejects an owner mismatch or any malformed persisted lifetime field.
pub fn decode_row(
    row: RawLifetimeRow<'_>,
    expected_owner: &[u8],
) -> Result<Lifetime, LifetimeReject> {
    if !byte_core::same_secret(row.owner, expected_owner)
        || row.deadline_ms < 0
        || row.startup_deadline_ms < 0
        || row.bytes <= 0
        || row.deadline_ms > row.max_deadline_ms
        || !started_within_deadline(row.started_at_ms, row.deadline_ms)
    {
        return Err(LifetimeReject::CorruptLifetime);
    }
    Ok(Lifetime {
        started_at_ms: row.started_at_ms,
        deadline_ms: row.deadline_ms,
        max_deadline_ms: row.max_deadline_ms,
        startup_deadline_ms: row.startup_deadline_ms,
        disk_bytes: row.bytes.cast_unsigned(),
    })
}

/// # Errors
/// Rejects a negative service clock.
pub fn check_clock(now_ms: i64) -> Result<(), LifetimeReject> {
    if now_ms < 0 {
        Err(LifetimeReject::InvalidClock)
    } else {
        Ok(())
    }
}

/// # Errors
/// Rejects missing, foreign-owned, or non-Running records in that order.
pub fn check_record(
    actor_tenant: Option<u64>,
    record: Option<(u64, LifetimePhase)>,
) -> Result<u64, LifetimeReject> {
    let (owner, phase) = record.ok_or(LifetimeReject::NotRunning)?;
    match actor_tenant {
        Some(tenant) if tenant == owner => {}
        _ => return Err(LifetimeReject::Unauthorized),
    }
    if !matches!(phase, LifetimePhase::Running) {
        return Err(LifetimeReject::NotRunning);
    }
    Ok(owner)
}

/// # Errors
/// Rejects a missing lifetime or a deadline at or before the service clock.
pub fn check_live(now_ms: i64, lifetime: Option<Lifetime>) -> Result<Lifetime, LifetimeReject> {
    let lifetime = lifetime.ok_or(LifetimeReject::MissingLifetime)?;
    if lifetime.deadline_ms <= now_ms {
        return Err(LifetimeReject::Expired);
    }
    Ok(lifetime)
}
