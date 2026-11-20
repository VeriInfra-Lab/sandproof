#![forbid(unsafe_code)]
//! Transparent calls only; no duplicate observation algorithm.
pub fn matches(
    observation: &sandproof_protocol::lifecycle::CreateObservation,
    original: &sandproof_protocol::lifecycle::AcceptedLifecycleIntent,
) -> bool {
    observation.matches_request(original)
}
