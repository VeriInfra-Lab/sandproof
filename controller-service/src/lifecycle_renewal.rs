//! Pure deletion rule used by the original durable renewal transaction.
//! The caller supplies actual fields read under the same authority; returning
//! true grants no native work, time extension, resource release or fresh Ready.
use sandproof_protocol::lifecycle::{AcceptedLifecycleIntent, LifecycleOperation, RenewalConfirmation};

/// Only the previous exact, applied SDK checkpoint can be discarded. Public,
/// current, unapplied and native/source-pinned records remain intact.
#[must_use]
pub fn can_discard_confirmed_sdk(
    request: &AcceptedLifecycleIntent,
    applied_end: Option<i64>,
    previous_effective_end: Option<i64>,
    local_operation: Option<sandbox_controller_core::OperationId>,
    confirmed: &RenewalConfirmation,
    current_version: u64,
    source_pinned: bool,
) -> bool {
    !source_pinned
        && request.request.binding.key.window.get() == 2
        && matches!(request.request.operation, LifecycleOperation::Renew { .. })
        && confirmed.matches_request(request)
        && applied_end == Some(confirmed.effective_end_at_ms)
        && previous_effective_end.is_some()
        && local_operation.is_none()
        && request.accepted_version != current_version
}
