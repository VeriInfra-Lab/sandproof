//! Original authoritative pending inputs and detached current Create queries.
//! Recorded never removes work or stands in for a current physical observation.
use crate::{AuthorityPort, CenterApplication, HostPort, NodeObservationPolicy, TemplateProvider};
use crate::application::authority_error;
use sandproof_api::PortError;
use sandproof_center_core::{DecisionError, SessionPhase};
use sandproof_center_store::authority::AuthorityError;
use sandproof_protocol::{TenantId, lifecycle::{AcceptedLifecycleIntent, CreateObservation, CurrentCreateReadiness, LifecycleOperation, SessionId}};

impl<A: AuthorityPort, H: HostPort, T: TemplateProvider> CenterApplication<A, H, T> {
    /// The protected existing node clock/freshness policy is reused; setting it
    /// does not provision a profile or install a public route.
    /// # Errors
    /// Zero/unbounded/mismatched time policy remains refused.
    pub fn with_lifecycle_observation_policy(mut self, policy: NodeObservationPolicy) -> Result<Self, PortError> {
        policy.validate(self.config.poll_interval)?;
        self.lifecycle_observation_policy = Some(policy);
        Ok(self)
    }

    pub(crate) fn current_create_observation(&self, intent: &AcceptedLifecycleIntent) -> Result<CreateObservation, PortError> {
        let policy = self.lifecycle_observation_policy.ok_or(PortError::Busy)?;
        let before = self.now()?;
        let view = self.hosts.observe_lifecycle_create(intent, self.config.reply_timeout)?;
        let after = self.now()?;
        if !view.matches_request(intent) { return Err(PortError::Busy); }
        let CurrentCreateReadiness::Observed { facts } = &view.readiness else { return Err(PortError::Busy); };
        if policy.healthy_until(before, after, facts.observed_at_ms).is_none() || after >= facts.end_at_ms {
            return Err(PortError::Busy);
        }
        Ok(view)
    }

    pub(crate) fn current_lifecycle_intent(&self, owner: &TenantId, session: &SessionId) -> Result<(AcceptedLifecycleIntent, SessionPhase), PortError> {
        let state = self.snapshot()?;
        let record = state.lifecycle_session(owner, session).ok_or(PortError::NotFound)?;
        let current = record.operations().last().ok_or(PortError::OutcomeUnknown)?.intent().clone();
        Ok((current, record.phase()))
    }

    /// Exactly one finite current-input snapshot; no queue or new operation.
    /// Node Running may already be committed even when its result was never
    /// published at the center. Re-observe it; never ask for another Create.
    /// # Errors
    /// Shared authority/unknown CAS stops; one host's Busy keeps its original
    /// responsibility and allows the other bounded entries a turn.
    pub(crate) fn reconcile_lifecycle_creates_once(&self) -> Result<usize, PortError> {
        let pending = self.authority.lock().map_err(|_| PortError::OutcomeUnknown)?
            .lifecycle_dispatch_batch().map_err(authority_error)?;
        let mut confirmed = 0;
        for intent in pending {
            if !matches!(intent.request.operation, LifecycleOperation::Create { .. }) { continue; }
            let Ok(received) = self.hosts.record_lifecycle_v3(&intent, self.config.reply_timeout) else { continue; };
            if !received.matches(&intent) || !received.current { continue; }
            let Ok(observation) = self.current_create_observation(&intent) else { continue; };
            let result = self.authority.lock().map_err(|_| PortError::OutcomeUnknown)?
                .accept_lifecycle_create_observation(&observation);
            match result {
                Ok(()) => confirmed += 1,
                Err(AuthorityError::Decision(DecisionError::Stale | DecisionError::Conflict | DecisionError::Unavailable | DecisionError::NotFound)) => {},
                Err(error) => return Err(authority_error(error)),
            }
        }
        Ok(confirmed)
    }
}
