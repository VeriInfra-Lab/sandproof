//! Fresh original execution observation after an immutable failed Unload.
use crate::application::authority_error;
use crate::lifecycle_dispatch::time_left;
use crate::{AuthorityPort, CenterApplication, HostPort, TemplateProvider};
use sandproof_api::PortError;
use sandproof_center_core::SessionPhase;
use sandproof_protocol::lifecycle::{
    AcceptedLifecycleIntent, CurrentCreateReadiness, CurrentReadyFacts, CurrentResumeReadiness, LifecycleOperation,
    PublicSessionPhase, PublicSessionView, SessionId,
};

impl<H: HostPort, T: TemplateProvider>
    CenterApplication<crate::business::SharedBusinessAuthority, H, T>
{
    fn observe_continued_before(
        &self,
        origin: &AcceptedLifecycleIntent,
        intent: &AcceptedLifecycleIntent,
        deadline: std::time::Instant,
    ) -> Result<CurrentReadyFacts, PortError> {
        let policy = self.lifecycle_observation_policy.ok_or(PortError::Busy)?;
        let before = self.now()?;
        let budget = time_left(deadline)?;
        // The existing authenticated origin observation independently verifies
        // its retained failed Unload and the actual same candidate on the host.
        let facts = match origin.request.operation {
            LifecycleOperation::Create { .. } => {
                let view = self.hosts.observe_lifecycle_create(origin, budget)?;
                if !view.matches_request(origin) || view.current_request != *intent {
                    return Err(PortError::Busy);
                }
                let CurrentCreateReadiness::Observed { facts } = view.readiness else {
                    return Err(PortError::Busy);
                };
                facts
            }
            LifecycleOperation::Resume { .. } => {
                let view = self.hosts.observe_lifecycle_resume(origin, budget)?;
                if !view.matches_request(origin) || view.current_request != *intent {
                    return Err(PortError::Busy);
                }
                let CurrentResumeReadiness::Observed { facts } = view.readiness else {
                    return Err(PortError::Busy);
                };
                facts
            }
            _ => return Err(PortError::Busy),
        };
        let after = self.now()?;
        time_left(deadline)?;
        if policy.healthy_until(before, after, facts.observed_at_ms).is_none()
            || after >= facts.end_at_ms
        {
            return Err(PortError::Busy);
        }
        Ok(facts)
    }

    pub(crate) fn query_continued(
        &self,
        owner: &sandproof_protocol::TenantId,
        session: &SessionId,
        intent: &AcceptedLifecycleIntent,
    ) -> Result<PublicSessionView, PortError> {
        let deadline = self.reply_deadline()?;
        let snapshot = self.snapshot()?;
        let record = snapshot.lifecycle_session(owner, session).ok_or(PortError::NotFound)?;
        let current = record.operations().last().ok_or(PortError::Busy)?;
        if current.intent() != intent || record.phase() != SessionPhase::Running {
            return Err(PortError::Busy);
        }
        let saved = current.unloaded().ok_or(PortError::Busy)?.clone();
        let prior = saved.outcome.continued().ok_or(PortError::Busy)?;
        let origin = record.current_execution_origin().ok_or(PortError::Busy)?.clone();
        let facts = self.observe_continued_before(&origin, intent, deadline)?;
        let after = self.now()?;
        if after >= facts.end_at_ms
            || facts.started_at_ms != prior.started_at_ms
            || facts.end_at_ms != prior.end_at_ms
            || facts.max_end_at_ms != prior.max_end_at_ms
            || facts.startup_deadline_ms != prior.startup_deadline_ms
        {
            return Err(PortError::Busy);
        }
        let mut authority = self.authority.lock().map_err(|_| PortError::OutcomeUnknown)?;
        // Reaffirm the SAME immutable historical outcome in the original CAS.
        // Fresh readiness is detached evidence, never a replacement saved Ready.
        authority.accept_lifecycle_unload_observation(&saved, after).map_err(authority_error)?;
        let current = authority.public_lifecycle_record(owner, session)?;
        let operation = current.operations().last().ok_or(PortError::Busy)?;
        if operation.intent() != intent || operation.unloaded() != Some(&saved)
            || current.current_execution_origin() != Some(&origin)
            || current.phase() != SessionPhase::Running || self.now()? >= facts.end_at_ms
        {
            return Err(PortError::Busy);
        }
        time_left(deadline)?;
        Ok(PublicSessionView {
            session: session.clone(),
            operation: intent.request.binding.key.clone(),
            current_version: intent.accepted_version,
            phase: PublicSessionPhase::Running,
            unload_outcome: Some(saved.outcome),
            create_refusal: None,
        })
    }
}
