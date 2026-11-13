#![forbid(unsafe_code)]

//! Only transparent calls to actual production protocol methods.
use sandproof_protocol::internal::NodeIdentity;
use sandproof_protocol::lifecycle::{
    AcceptedIntentError, AcceptedLifecycleIntent, AuthenticatedCenterContext, LifecycleRequest,
    RecordedLifecycleIntent, ValidationError,
};

pub fn request_validate(
    request: &LifecycleRequest,
    expected: &NodeIdentity,
) -> Result<(), ValidationError> {
    request.validate(expected)
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

// Load the original durable crate so Charon can select its private lookup root.
pub use sandproof_controller_service::JournalError;
