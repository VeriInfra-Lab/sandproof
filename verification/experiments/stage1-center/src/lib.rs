//! Thin roots calling the actual production functions, not a second algorithm.
use sandproof_center_core::{
    CenterState, DecisionError, LifecycleOperationRecord, LifecycleSubmission,
};
use sandproof_protocol::TenantId;
use sandproof_protocol::lifecycle::AcceptedLifecycleIntent;
use sandproof_protocol::lifecycle::TerminalLifecycleObservation;

pub fn version_next(expected: u64, current: u64) -> Result<u64, DecisionError> {
    sandproof_center_core::next_lifecycle_version(expected, current)
}
pub fn replay(
    record: &LifecycleOperationRecord,
    submission: &LifecycleSubmission,
) -> Result<AcceptedLifecycleIntent, DecisionError> {
    record.replay(submission)
}
pub fn accept(
    state: &CenterState,
    owner: &TenantId,
    submission: &LifecycleSubmission,
    now_ms: u64,
) -> Result<(CenterState, AcceptedLifecycleIntent), DecisionError> {
    state.accept_lifecycle(owner, submission, now_ms)
}

/// Direct production call-through, including actual recovery validation,
/// first-result storage, one-time retention initialization and refusal rules.
pub fn terminal_result(
    state: &CenterState,
    result: &TerminalLifecycleObservation,
    now_ms: u64,
) -> Result<CenterState, DecisionError> {
    state.accept_lifecycle_terminal_observation(result, now_ms)
}

/// Original pure SDK confirmation/prune, including the real recovery checks.
pub fn renewal_result(
    state: &CenterState,
    result: &sandproof_protocol::lifecycle::ConfirmedRenewal,
) -> Result<CenterState, DecisionError> {
    state.confirm_lifecycle_renewal(result)
}

/// The exact rule called by the original SQL adapter, without durable features.
pub fn renewal_rule(
    request: &AcceptedLifecycleIntent,
    applied_end: Option<i64>,
    previous_end: Option<i64>,
    local_operation: Option<sandbox_controller_core::OperationId>,
    checkpoint: &sandproof_protocol::lifecycle::RenewalConfirmation,
    current_version: u64,
    source_pinned: bool,
) -> bool {
    sandproof_controller_service::lifecycle_renewal::can_discard_confirmed_sdk(
        request,
        applied_end,
        previous_end,
        local_operation,
        checkpoint,
        current_version,
        source_pinned,
    )
}
