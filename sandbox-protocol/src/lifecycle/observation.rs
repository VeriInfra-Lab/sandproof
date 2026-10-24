//! Explicit projection of one original application's current Create query.
//! There are no native IDs, credentials, paths or invented host sequence here.
use super::{AcceptedLifecycleIntent, LifecycleOperation};
use serde::{Deserialize, Serialize};

/// A durable refusal of the original Create before native admission. This is
/// never inferred from Busy, an absent mapping, a timeout or an I/O failure.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CreateRefusalReason {
    Capacity,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CurrentReadyFacts {
    pub started_at_ms: u64,
    pub end_at_ms: u64,
    pub max_end_at_ms: u64,
    pub startup_deadline_ms: u64,
    pub observed_at_ms: u64,
}
impl CurrentReadyFacts {
    /// Preserve the original execution and total lifetime after a normalized
    /// update. Actual application of the current Renew is a separate journal
    /// provider obligation; merely satisfying these values cannot prove it.
    #[must_use]
    pub fn matches_renewed(
        &self,
        original: &AcceptedLifecycleIntent,
        current: &AcceptedLifecycleIntent,
    ) -> bool {
        let LifecycleOperation::Create {
            create,
            target_incarnation,
        } = &original.request.operation
        else {
            return false;
        };
        let LifecycleOperation::Renew {
            incarnation,
            end_at_ms,
            extend_only,
        } = &current.request.operation
        else {
            return false;
        };
        let Ok(end) = u64::try_from(*end_at_ms) else {
            return false;
        };
        *incarnation == *target_incarnation
            && self.startup_deadline_ms == create.startup_deadline_ms
            && self.started_at_ms <= self.startup_deadline_ms
            && self.started_at_ms.checked_add(create.max_lifetime_ms) == Some(self.max_end_at_ms)
            && self.started_at_ms <= self.observed_at_ms
            && self.observed_at_ms < self.end_at_ms
            && self.end_at_ms <= self.max_end_at_ms
            && if *extend_only {
                self.end_at_ms >= end
            } else {
                self.end_at_ms == end
            }
    }
    /// Shape and original-lifetime checks, not proof of physical observation.
    #[must_use]
    pub fn matches_create(&self, intent: &AcceptedLifecycleIntent) -> bool {
        let LifecycleOperation::Create { create, .. } = &intent.request.operation else {
            return false;
        };
        self.startup_deadline_ms == create.startup_deadline_ms
            && self.started_at_ms <= self.startup_deadline_ms
            && self.started_at_ms.checked_add(create.timeout_ms) == Some(self.end_at_ms)
            && self.started_at_ms.checked_add(create.max_lifetime_ms) == Some(self.max_end_at_ms)
            && self.started_at_ms <= self.observed_at_ms
            && self.observed_at_ms < self.end_at_ms
            && self.end_at_ms <= self.max_end_at_ms
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case", deny_unknown_fields)]
pub enum CurrentCreateReadiness {
    NotCurrent,
    NotRunning,
    Expired,
    Pending,
    /// Bound to this observation's full original request. The same durable
    /// refusal remains queryable while its current Destroy is being settled.
    Refused {
        reason: CreateRefusalReason,
    },
    Observed {
        facts: CurrentReadyFacts,
    },
}

/// Historical success and current readiness are independent. False historical
/// success does not mean no execution/effect, and none of these values releases.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CreateObservation {
    pub request: AcceptedLifecycleIntent,
    pub current_request: AcceptedLifecycleIntent,
    pub historical_create_succeeded: bool,
    pub readiness: CurrentCreateReadiness,
}
impl CreateObservation {
    /// Full caller/response binding and finite value checks only. The sender,
    /// actual readonly call and all own checks require their original providers.
    #[must_use]
    pub fn matches_request(&self, original: &AcceptedLifecycleIntent) -> bool {
        let requested = &original.request.binding;
        let current = &self.current_request.request.binding;
        if self.request != *original
            || original.center.deployment != requested.node.deployment_id
            || !matches!(
                original.request.operation,
                LifecycleOperation::Create { .. }
            )
            || self.current_request.center != original.center
            || self.current_request.retention != original.retention
            || current.node != requested.node
            || current.session != requested.session
            || current.key.owner != requested.key.owner
            || current.key.deployment != requested.key.deployment
            || self
                .current_request
                .request
                .validate(&requested.node)
                .is_err()
            || self.current_request.accepted_version < original.accepted_version
            || original.accepted_version == 0
        {
            return false;
        }
        if matches!(self.readiness, CurrentCreateReadiness::Refused { .. }) {
            if self.historical_create_succeeded
                || original.accepted_version != 1
                || original.accepted_version_for_request() != Some(1)
                || original.request.binding.expected != super::ExpectedSession::Absent
                || original.request.validate(&requested.node).is_err()
            {
                return false;
            }
            return self.current_request == *original
                || (matches!(
                    self.current_request.request.operation,
                    LifecycleOperation::Destroy {}
                ) && self.current_request.accepted_version > original.accepted_version
                    && self.current_request.accepted_version_for_request()
                        == Some(self.current_request.accepted_version)
                    && current.key != requested.key);
        }
        if self.current_request != *original {
            if matches!(
                self.current_request.request.operation,
                LifecycleOperation::Unload { .. }
            ) && self.readiness != CurrentCreateReadiness::NotCurrent
            {
                return continued_request(original, &self.current_request)
                    && match &self.readiness {
                        CurrentCreateReadiness::Observed { facts } => {
                            self.historical_create_succeeded && continued_facts(facts)
                        }
                        CurrentCreateReadiness::NotRunning
                        | CurrentCreateReadiness::Expired
                        | CurrentCreateReadiness::Pending => true,
                        _ => false,
                    };
            }
            if matches!(
                self.current_request.request.operation,
                LifecycleOperation::Renew { .. }
            ) {
                if self.current_request.accepted_version <= original.accepted_version
                    || self.current_request.accepted_version_for_request()
                        != Some(self.current_request.accepted_version)
                {
                    return false;
                }
                return match &self.readiness {
                    CurrentCreateReadiness::Observed { facts } => {
                        self.historical_create_succeeded
                            && facts.matches_renewed(original, &self.current_request)
                    }
                    CurrentCreateReadiness::NotRunning
                    | CurrentCreateReadiness::Expired
                    | CurrentCreateReadiness::Pending => true,
                    CurrentCreateReadiness::NotCurrent | CurrentCreateReadiness::Refused { .. } => {
                        false
                    }
                };
            }
            return self.current_request.accepted_version > original.accepted_version
                && self.readiness == CurrentCreateReadiness::NotCurrent;
        }
        match &self.readiness {
            CurrentCreateReadiness::NotCurrent | CurrentCreateReadiness::Refused { .. } => false,
            CurrentCreateReadiness::Observed { facts } => {
                self.historical_create_succeeded && facts.matches_create(original)
            }
            CurrentCreateReadiness::NotRunning
            | CurrentCreateReadiness::Expired
            | CurrentCreateReadiness::Pending => true,
        }
    }
}

#[cfg(test)]
#[path = "observation_tests.rs"]
mod tests;

// Shape only: both authorities must independently retain OriginalContinued.
pub(super) fn continued_request(
    origin: &AcceptedLifecycleIntent,
    current: &AcceptedLifecycleIntent,
) -> bool {
    let (LifecycleOperation::Create {
        target_incarnation: target,
        ..
    }
    | LifecycleOperation::Resume {
        target_incarnation: target,
        ..
    }) = origin.request.operation
    else {
        return false;
    };
    let LifecycleOperation::Unload {
        source_incarnation, ..
    } = current.request.operation
    else {
        return false;
    };
    target == source_incarnation
        && current.accepted_version > origin.accepted_version
        && current.request.binding.key != origin.request.binding.key
        && current.accepted_version_for_request() == Some(current.accepted_version)
        && origin.accepted_version_for_request() == Some(origin.accepted_version)
}

pub(super) fn continued_facts(facts: &CurrentReadyFacts) -> bool {
    facts.started_at_ms <= facts.startup_deadline_ms
        && facts.started_at_ms <= facts.observed_at_ms
        && facts.observed_at_ms < facts.end_at_ms
        && facts.end_at_ms <= facts.max_end_at_ms
}
