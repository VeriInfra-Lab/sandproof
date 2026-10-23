//! Terminal query data; only the original physical/journal provider can supply
//! Cleaned. Intent reception, elapsed time and native slot release alone cannot.
use super::{AcceptedLifecycleIntent, LifecycleOperation};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TerminalStatus {
    Pending,
    Cleaned,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TerminalLifecycleObservation {
    pub request: AcceptedLifecycleIntent,
    pub status: TerminalStatus,
}
impl TerminalLifecycleObservation {
    /// Complete request binding only. This does not prove that any resource
    /// was released, nor that the center has committed or confirmed the result.
    #[must_use]
    pub fn matches_request(&self, request: &AcceptedLifecycleIntent) -> bool {
        self.request == *request
            && matches!(request.request.operation, LifecycleOperation::Destroy {})
            && request.accepted_version > 0
            && request.accepted_version_for_request() == Some(request.accepted_version)
    }
}
