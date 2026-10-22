//! Thin roots calling the actual production functions, not a second algorithm.
use sandproof_center_core::{
    CenterState, DecisionError, LifecycleOperationRecord, LifecycleSubmission,
};
use sandproof_protocol::TenantId;
use sandproof_protocol::lifecycle::AcceptedLifecycleIntent;

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
