#![forbid(unsafe_code)]

//! Transparent calls to the real protocol-4 result and confirmation methods.
use sandproof_protocol::internal::NodeIdentity;
use sandproof_protocol::lifecycle::{
    AcceptedIntentError, AcceptedLifecycleIntent, AuthenticatedCenterContext,
    ClosedLifecycleWindow, ConfirmedLifecycleResult, ConfirmedRenewal, LifecycleRequest,
    LifecycleWindowClose, RecordedLifecycleIntent, RenewalConfirmation,
    TerminalLifecycleObservation, ValidationError,
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

pub fn terminal_matches(
    result: &TerminalLifecycleObservation,
    request: &AcceptedLifecycleIntent,
) -> bool {
    result.matches_request(request)
}

pub fn confirmed_matches(
    ack: &ConfirmedLifecycleResult,
    result: &TerminalLifecycleObservation,
) -> bool {
    ack.matches(result)
}

pub fn window_close_valid(request: &LifecycleWindowClose) -> bool {
    request.is_terminal_result()
}

pub fn closed_matches(ack: &ClosedLifecycleWindow, request: &LifecycleWindowClose) -> bool {
    ack.matches(request)
}

pub fn renewal_valid(result: &RenewalConfirmation) -> bool {
    result.is_sdk_renewal()
}

pub fn renewal_matches(result: &RenewalConfirmation, request: &AcceptedLifecycleIntent) -> bool {
    result.matches_request(request)
}

pub fn renewal_confirmed_matches(ack: &ConfirmedRenewal, result: &RenewalConfirmation) -> bool {
    ack.matches(result)
}
