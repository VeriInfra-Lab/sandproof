//! Explicit projection of one original application's current Create query.
//! There are no native IDs, credentials, paths or invented host sequence here.
use super::{AcceptedLifecycleIntent, LifecycleOperation};
use serde::{Deserialize, Serialize};

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
    pub fn matches_renewed(&self, original: &AcceptedLifecycleIntent, current: &AcceptedLifecycleIntent) -> bool {
        let LifecycleOperation::Create { create, target_incarnation } = &original.request.operation else { return false; };
        let LifecycleOperation::Renew { incarnation, end_at_ms, extend_only } = &current.request.operation else { return false; };
        let Ok(end) = u64::try_from(*end_at_ms) else { return false; };
        *incarnation == *target_incarnation
            && self.startup_deadline_ms == create.startup_deadline_ms
            && self.started_at_ms <= self.startup_deadline_ms
            && self.started_at_ms.checked_add(create.max_lifetime_ms) == Some(self.max_end_at_ms)
            && self.started_at_ms <= self.observed_at_ms
            && self.observed_at_ms < self.end_at_ms
            && self.end_at_ms <= self.max_end_at_ms
            && if *extend_only { self.end_at_ms >= end } else { self.end_at_ms == end }
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
    Observed { facts: CurrentReadyFacts },
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
            || !matches!(original.request.operation, LifecycleOperation::Create { .. })
            || self.current_request.center != original.center
            || self.current_request.retention != original.retention
            || current.node != requested.node
            || current.session != requested.session
            || current.key.owner != requested.key.owner
            || current.key.deployment != requested.key.deployment
            || self.current_request.request.validate(&requested.node).is_err()
            || self.current_request.accepted_version < original.accepted_version
            || original.accepted_version == 0
        {
            return false;
        }
        if self.current_request != *original {
            if matches!(self.current_request.request.operation, LifecycleOperation::Renew { .. }) {
                if self.current_request.accepted_version <= original.accepted_version
                    || self.current_request.accepted_version_for_request() != Some(self.current_request.accepted_version)
                { return false; }
                return match &self.readiness {
                    CurrentCreateReadiness::Observed { facts } => self.historical_create_succeeded
                        && facts.matches_renewed(original, &self.current_request),
                    CurrentCreateReadiness::NotRunning | CurrentCreateReadiness::Expired
                    | CurrentCreateReadiness::Pending => true,
                    CurrentCreateReadiness::NotCurrent => false,
                };
            }
            return self.current_request.accepted_version > original.accepted_version
                && self.readiness == CurrentCreateReadiness::NotCurrent;
        }
        match &self.readiness {
            CurrentCreateReadiness::NotCurrent => false,
            CurrentCreateReadiness::Observed { facts } => {
                self.historical_create_succeeded && facts.matches_create(original)
            }
            CurrentCreateReadiness::NotRunning
            | CurrentCreateReadiness::Expired
            | CurrentCreateReadiness::Pending => true,
        }
    }
}
