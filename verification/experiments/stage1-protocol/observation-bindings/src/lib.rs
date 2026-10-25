#![forbid(unsafe_code)]
//! Transparent current-protocol observation roots; no resource or journal model.
use sandproof_protocol::internal::NodeIdentity;
use sandproof_protocol::lifecycle::{
    AcceptedIntentError, AcceptedLifecycleIntent, AuthenticatedCenterContext, CurrentReadyFacts,
    LifecycleRequest, RecordedLifecycleIntent, ResumeObservation, UnloadedFacts, ValidationError,
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
pub fn ready_matches_create(facts: &CurrentReadyFacts, original: &AcceptedLifecycleIntent) -> bool {
    facts.matches_create(original)
}
pub fn ready_matches_resumed(
    facts: &CurrentReadyFacts,
    original: &AcceptedLifecycleIntent,
    source: &UnloadedFacts,
) -> bool {
    facts.matches_resumed(original, source)
}
pub fn ready_matches_resumed_renewal(
    facts: &CurrentReadyFacts,
    original: &AcceptedLifecycleIntent,
    current: &AcceptedLifecycleIntent,
    source: &UnloadedFacts,
) -> bool {
    facts.matches_resumed_renewal(original, current, source)
}
pub fn resume_matches_request(
    observation: &ResumeObservation,
    original: &AcceptedLifecycleIntent,
) -> bool {
    observation.matches_request(original)
}

pub fn create_observation_equal(
    left: &sandproof_protocol::lifecycle::CreateObservation,
    right: &sandproof_protocol::lifecycle::CreateObservation,
) -> bool {
    left == right
}
pub fn create_matches_request(
    value: &sandproof_protocol::lifecycle::CreateObservation,
    original: &AcceptedLifecycleIntent,
) -> bool {
    value.matches_request(original)
}
pub fn ready_matches_create_renewal(
    value: &CurrentReadyFacts,
    original: &AcceptedLifecycleIntent,
    current: &AcceptedLifecycleIntent,
) -> bool {
    value.matches_renewed(original, current)
}

// Keep the same actual standard conversion reachable in correct and wrong views.
// This root does not replace any production guard or its negative variant.
pub fn create_signed_deadline(value: i64) -> Result<u64, std::num::TryFromIntError> {
    u64::try_from(value)
}
