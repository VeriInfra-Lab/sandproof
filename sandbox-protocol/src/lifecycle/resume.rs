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
    Observed {
        facts: CurrentReadyFacts,
    },
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
        if self.request != *original {
            return false;
        }
        let LifecycleOperation::Resume { .. } = &original.request.operation else {
            return false;
        };
        let Ok(()) = original.request.validate(&binding.node) else {
            return false;
        };
        if original.center.deployment != binding.node.deployment_id
            || original.accepted_version == 0
            || original.accepted_version_for_request() != Some(original.accepted_version)
            || !original.retention.valid_for(4096)
            || self.current_request.center != original.center
            || self.current_request.retention != original.retention
            || current.node != binding.node
            || current.session != binding.session
            || current.key.owner != binding.key.owner
            || current.key.deployment != binding.key.deployment
        {
            return false;
        }
        let Ok(()) = self.current_request.request.validate(&binding.node) else {
            return false;
        };
        if self.current_request.accepted_version_for_request()
            != Some(self.current_request.accepted_version)
        {
            return false;
        }
        if self.current_request != *original {
            if matches!(
                self.current_request.request.operation,
                LifecycleOperation::Unload { .. }
            ) && self.readiness != CurrentResumeReadiness::NotCurrent
            {
                return super::observation::continued_request(original, &self.current_request)
                    && match &self.readiness {
                        CurrentResumeReadiness::Observed { facts } => current_facts(facts),
                        CurrentResumeReadiness::NotRunning
                        | CurrentResumeReadiness::Expired
                        | CurrentResumeReadiness::Pending => true,
                        _ => false,
                    };
            }
            if let LifecycleOperation::Renew {
                incarnation,
                end_at_ms,
                extend_only,
            } = self.current_request.request.operation
            {
                let LifecycleOperation::Resume {
                    target_incarnation, ..
                } = original.request.operation
                else {
                    return false;
                };
                return self.current_request.accepted_version > original.accepted_version
                    && current.key != binding.key
                    && incarnation == target_incarnation
                    && match &self.readiness {
                        CurrentResumeReadiness::Observed { facts } => {
                            if end_at_ms < 0 {
                                false
                            } else {
                                // Nonnegative i64 values fit exactly in u64.
                                #[allow(
                                    clippy::cast_sign_loss,
                                    reason = "negative lifetime rejected above"
                                )]
                                let end = end_at_ms as u64;
                                current_facts(facts)
                                    && if extend_only {
                                        facts.end_at_ms >= end
                                    } else {
                                        facts.end_at_ms == end
                                    }
                            }
                        }
                        CurrentResumeReadiness::NotCurrent => false,
                        _ => true,
                    };
            }
            return self.current_request.accepted_version > original.accepted_version
                && current.key != binding.key
                && self.readiness == CurrentResumeReadiness::NotCurrent;
        }
        match &self.readiness {
            CurrentResumeReadiness::NotCurrent => false,
            CurrentResumeReadiness::Observed { facts } => current_facts(facts),
            CurrentResumeReadiness::NotRunning
            | CurrentResumeReadiness::Expired
            | CurrentResumeReadiness::Pending
            | CurrentResumeReadiness::StateLost => true,
        }
    }
}

fn current_facts(facts: &CurrentReadyFacts) -> bool {
    facts.started_at_ms <= facts.startup_deadline_ms
        && facts.started_at_ms <= facts.observed_at_ms
        && facts.observed_at_ms < facts.end_at_ms
        && facts.end_at_ms <= facts.max_end_at_ms
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
        self.matches_resume_source(original, source)
            && self.end_at_ms == source.end_at_ms
            && current_facts(self)
    }

    fn matches_resume_source(
        &self,
        original: &AcceptedLifecycleIntent,
        source: &UnloadedFacts,
    ) -> bool {
        let LifecycleOperation::Resume {
            source_incarnation,
            capture,
            manifest_digest,
            ..
        } = &original.request.operation
        else {
            return false;
        };
        source.source_incarnation == *source_incarnation
            && source.capture == *capture
            && source.manifest_digest == *manifest_digest
            && self.started_at_ms == source.started_at_ms
            && self.max_end_at_ms == source.max_end_at_ms
            && self.startup_deadline_ms == source.startup_deadline_ms
    }

    /// The original resumed execution may be renewed without changing its
    /// source, start or total limit. Actual monotonic renewal admission and
    /// application still belong to the center and original local journal.
    #[must_use]
    pub fn matches_resumed_renewal(
        &self,
        original: &AcceptedLifecycleIntent,
        current: &AcceptedLifecycleIntent,
        source: &UnloadedFacts,
    ) -> bool {
        let LifecycleOperation::Resume {
            target_incarnation, ..
        } = original.request.operation
        else {
            return false;
        };
        let LifecycleOperation::Renew {
            incarnation,
            end_at_ms,
            extend_only,
        } = current.request.operation
        else {
            return false;
        };
        if end_at_ms < 0 {
            return false;
        }
        // Nonnegative i64 values fit exactly in u64.
        #[allow(clippy::cast_sign_loss, reason = "negative lifetime rejected above")]
        let end = end_at_ms as u64;
        self.matches_resume_source(original, source)
            && incarnation == target_incarnation
            && current_facts(self)
            && if extend_only {
                self.end_at_ms >= end
            } else {
                self.end_at_ms == end
            }
    }
}

#[cfg(test)]
#[path = "resume_tests.rs"]
mod tests;
