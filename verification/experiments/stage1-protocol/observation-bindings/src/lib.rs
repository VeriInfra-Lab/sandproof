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
