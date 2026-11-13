#![forbid(unsafe_code)]
//! Transparent native entry for the actual full Resume observation guard.
pub fn matches(
    observation: &sandproof_protocol::lifecycle::ResumeObservation,
    original: &sandproof_protocol::lifecycle::AcceptedLifecycleIntent,
) -> bool {
    observation.matches_request(original)
}
