#![forbid(unsafe_code)]
//! Transparent decoded access guard; original journal authorization is separate.
use sandproof_protocol::internal::NodeIdentity;
use sandproof_protocol::lifecycle::{
    AcceptedIntentError, AcceptedLifecycleIntent, AccessIdentityError, AuthenticatedCenterContext,
    LifecycleAccessIdentity, LifecycleRequest, RecordedLifecycleIntent, ValidationError,
};

pub fn request_validate(
    request: &LifecycleRequest,
    node: &NodeIdentity,
) -> Result<(), ValidationError> {
    request.validate(node)
}
pub fn replay_matches(request: &LifecycleRequest, original: &LifecycleRequest) -> bool {
    request.replay_matches(original)
}
pub fn accepted_validate(
    intent: &AcceptedLifecycleIntent,
    context: &AuthenticatedCenterContext,
) -> Result<(), AcceptedIntentError> {
    intent.validate(context)
}
pub fn recorded_matches(
    recorded: &RecordedLifecycleIntent,
    original: &AcceptedLifecycleIntent,
) -> bool {
    recorded.matches(original)
}
pub fn access_validate(
    identity: &LifecycleAccessIdentity,
    context: &AuthenticatedCenterContext,
) -> Result<(), AccessIdentityError> {
    identity.validate(context)
}
