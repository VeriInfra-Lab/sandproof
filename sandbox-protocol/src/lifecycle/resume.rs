//! Current execution observations for the original retained Resume request.
use super::{AcceptedLifecycleIntent, CurrentReadyFacts, LifecycleOperation, UnloadedFacts};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case", deny_unknown_fields)]
pub enum CurrentResumeReadiness {
    NotCurrent,
    NotRunning,
    Expired,
    Pending,
    /// The provider must establish that the same possibly activated candidate
    /// has died. Missing replies, a timeout or an absent observation do not.
    StateLost,
    Observed { facts: CurrentReadyFacts },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResumeObservation {
    pub request: AcceptedLifecycleIntent,
    pub current_request: AcceptedLifecycleIntent,
    pub readiness: CurrentResumeReadiness,
}

impl ResumeObservation {
    /// Complete wire binding and value shape only. The original journal and
    /// actual candidate observation supply readiness, loss and material facts.
    #[must_use]
    pub fn matches_request(&self, original: &AcceptedLifecycleIntent) -> bool {
        let binding = &original.request.binding;
        let current = &self.current_request.request.binding;
        if self.request != *original
            || !matches!(original.request.operation, LifecycleOperation::Resume { .. })
            || original.request.validate(&binding.node).is_err()
            || original.center.deployment != binding.node.deployment_id
            || original.accepted_version == 0
            || original.accepted_version_for_request() != Some(original.accepted_version)
            || !original.retention.valid_for(4096)
            || self.current_request.center != original.center
            || self.current_request.retention != original.retention
            || current.node != binding.node
            || current.session != binding.session
            || current.key.owner != binding.key.owner
            || current.key.deployment != binding.key.deployment
            || self.current_request.request.validate(&binding.node).is_err()
            || self.current_request.accepted_version_for_request()
                != Some(self.current_request.accepted_version)
        {
            return false;
        }
        if self.current_request != *original {
            return self.current_request.accepted_version > original.accepted_version
                && current.key != binding.key
                && self.readiness == CurrentResumeReadiness::NotCurrent;
        }
        match &self.readiness {
            CurrentResumeReadiness::NotCurrent => false,
            CurrentResumeReadiness::Observed { facts } => {
                facts.started_at_ms <= facts.startup_deadline_ms
                    && facts.started_at_ms <= facts.observed_at_ms
                    && facts.observed_at_ms < facts.end_at_ms
                    && facts.end_at_ms <= facts.max_end_at_ms
            }
            CurrentResumeReadiness::NotRunning
            | CurrentResumeReadiness::Expired
            | CurrentResumeReadiness::Pending
            | CurrentResumeReadiness::StateLost => true,
        }
    }
}

impl CurrentReadyFacts {
    /// A resumed execution keeps the complete original lifetime. The source
    /// must come from the authority's preceding Unloaded, not response input.
    #[must_use]
    pub fn matches_resumed(
        &self,
        original: &AcceptedLifecycleIntent,
        source: &UnloadedFacts,
    ) -> bool {
        let LifecycleOperation::Resume {
            source_incarnation, capture, manifest_digest, ..
        } = &original.request.operation else {
            return false;
        };
        source.source_incarnation == *source_incarnation
            && source.capture == *capture
            && source.manifest_digest == *manifest_digest
            && self.started_at_ms == source.started_at_ms
            && self.end_at_ms == source.end_at_ms
            && self.max_end_at_ms == source.max_end_at_ms
            && self.startup_deadline_ms == source.startup_deadline_ms
            && self.started_at_ms <= self.observed_at_ms
            && self.observed_at_ms < self.end_at_ms
    }
}

#[cfg(test)]
#[path = "resume_tests.rs"]
mod tests;
