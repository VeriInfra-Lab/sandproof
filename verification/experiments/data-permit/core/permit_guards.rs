//! Finite admission guards; callers retain the authority lock and query order.
//! These guards say nothing about the truth of the observations supplied.

#[must_use]
pub fn register_gate_allows(bounded: bool, gate_present: bool) -> bool {
    !bounded || gate_present
}

/// The caller reads the watch value only when this returns true.
#[must_use]
pub fn signal_needs_revocation_read(has_receivers: bool) -> bool {
    !has_receivers
}

#[must_use]
pub fn retain_after_revocation(revoked: bool) -> bool {
    !revoked
}

#[must_use]
pub fn create_signal_required(last_revoked: Option<bool>) -> bool {
    last_revoked.unwrap_or(true)
}

#[must_use]
pub fn dispatch_not_revoked(revoked: bool) -> bool {
    !revoked
}

#[must_use]
pub fn dispatch_configured(bounded: bool) -> bool {
    bounded
}

#[must_use]
pub fn dispatch_target_matches(
    current_vm: Option<u64>,
    target_vm: u64,
    generation: u64,
    target_generation: u64,
) -> bool {
    match current_vm {
        None => false,
        Some(vm) => vm == target_vm && generation == target_generation,
    }
}
