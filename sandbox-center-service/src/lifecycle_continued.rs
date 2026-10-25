//! Fresh original execution observation after an immutable failed Unload.
use crate::application::authority_error;
use crate::lifecycle_dispatch::{CurrentObservationError, time_left};
use crate::{AuthorityPort, CenterApplication, HostPort, TemplateProvider};
use sandproof_api::PortError;
use sandproof_center_core::SessionPhase;
use sandproof_protocol::lifecycle::{
    AcceptedLifecycleIntent, CurrentCreateReadiness, CurrentReadyFacts, CurrentResumeReadiness,
    LifecycleOperation, PublicSessionPhase, PublicSessionView, ResumeObservation, SessionId,
    UnloadObservation,
};

impl<H: HostPort, T: TemplateProvider>
    CenterApplication<crate::business::SharedBusinessAuthority, H, T>
{
    fn observe_continued_create_before(
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
            _ => return Err(PortError::Busy),
        };
        let after = self.now()?;
        time_left(deadline)?;
        if policy
            .healthy_until(before, after, facts.observed_at_ms)
            .is_none()
            || after >= facts.end_at_ms
        {
            return Err(PortError::Busy);
        }
        Ok(facts)
    }
}

impl<A: AuthorityPort, H: HostPort, T: TemplateProvider> CenterApplication<A, H, T> {
    fn save_continued_loss_before(
        &self,
        observation: &ResumeObservation,
        saved: &UnloadObservation,
        deadline: std::time::Instant,
    ) -> Result<(), PortError> {
        let prior = saved.outcome.continued().ok_or(PortError::Busy)?;
        if observation.readiness != CurrentResumeReadiness::StateLost
            || saved.request != observation.current_request
            || !observation.matches_request(&observation.request)
            || self.now()? >= prior.end_at_ms
        {
            return Err(PortError::Busy);
        }
        time_left(deadline)?;
        let owner = &observation.current_request.request.binding.key.owner;
        let session = &observation.current_request.request.binding.session;
        let mut authority = self
            .authority
            .lock()
            .map_err(|_| PortError::OutcomeUnknown)?;
        let state = AuthorityPort::snapshot(&mut *authority).map_err(authority_error)?;
        let current = state
            .lifecycle_session(owner, session)
            .ok_or(PortError::NotFound)?;
        let operation = current.operations().last().ok_or(PortError::Busy)?;
        if operation.intent() != &observation.current_request
            || operation.unloaded() != Some(saved)
            || current.current_execution_origin() != Some(&observation.request)
            || !matches!(
                current.phase(),
                SessionPhase::Running | SessionPhase::StateLost
            )
            || self.now()? >= prior.end_at_ms
        {
            return Err(PortError::Busy);
        }
        authority
            .accept_lifecycle_resume_observation(observation)
            .map_err(authority_error)?;
        let state = AuthorityPort::snapshot(&mut *authority).map_err(authority_error)?;
        let current = state
            .lifecycle_session(owner, session)
            .ok_or(PortError::NotFound)?;
        let operation = current.operations().last().ok_or(PortError::Busy)?;
        if operation.intent() != &observation.current_request
            || operation.unloaded() != Some(saved)
            || current.current_execution_origin() != Some(&observation.request)
            || current.phase() != SessionPhase::StateLost
        {
            return Err(PortError::Busy);
        }
        time_left(deadline)?;
        Ok(())
    }

    // This is consumed only by the existing per-intent reconciliation call.
    // Completed Unloads are not added to another background scanner.
    pub(crate) fn reconcile_continued_loss(
        &self,
        intent: &AcceptedLifecycleIntent,
        deadline: std::time::Instant,
    ) -> Result<bool, PortError> {
        let state = self.snapshot()?;
        let Some(record) = state.lifecycle_session(
            &intent.request.binding.key.owner,
            &intent.request.binding.session,
        ) else {
            return Ok(false);
        };
        let Some(current) = record.operations().last() else {
            return Ok(false);
        };
        let Some(saved) = current.unloaded() else {
            return Ok(false);
        };
        if current.intent() != intent
            || record.phase() != SessionPhase::Running
            || saved.outcome.continued().is_none()
            || record.current_execution_origin().is_none_or(|origin| {
                !matches!(origin.request.operation, LifecycleOperation::Resume { .. })
            })
        {
            return Ok(false);
        }
        let observation = match self.current_resume_observation_before(intent, deadline) {
            Ok(view) if view.readiness == CurrentResumeReadiness::StateLost => view,
            Err(CurrentObservationError::Shared(error)) => return Err(error),
            Ok(_) | Err(_) => return Ok(false),
        };
        match self.save_continued_loss_before(&observation, saved, deadline) {
            Ok(()) => Ok(true),
            Err(PortError::Busy | PortError::NotFound) => Ok(false),
            Err(error) => Err(error),
        }
    }
}

impl<H: HostPort, T: TemplateProvider>
    CenterApplication<crate::business::SharedBusinessAuthority, H, T>
{
    fn publish_continued_loss_before(
        &self,
        observation: &ResumeObservation,
        saved: &UnloadObservation,
        deadline: std::time::Instant,
    ) -> Result<PublicSessionView, PortError> {
        self.save_continued_loss_before(observation, saved, deadline)?;
        let owner = &observation.current_request.request.binding.key.owner;
        let session = &observation.current_request.request.binding.session;
        let authority = self
            .authority
            .lock()
            .map_err(|_| PortError::OutcomeUnknown)?;
        let current = authority.public_lifecycle_record(owner, session)?;
        let operation = current.operations().last().ok_or(PortError::Busy)?;
        if operation.intent() != &observation.current_request
            || operation.unloaded() != Some(saved)
            || current.current_execution_origin() != Some(&observation.request)
            || current.phase() != SessionPhase::StateLost
            || self.now()? >= saved.outcome.continued().ok_or(PortError::Busy)?.end_at_ms
        {
            return Err(PortError::Busy);
        }
        time_left(deadline)?;
        authority.query_public_lifecycle(owner, session)
    }

    pub(crate) fn query_continued(
        &self,
        owner: &sandproof_protocol::TenantId,
        session: &SessionId,
        intent: &AcceptedLifecycleIntent,
    ) -> Result<PublicSessionView, PortError> {
        let deadline = self.reply_deadline()?;
        let snapshot = self.snapshot()?;
        let record = snapshot
            .lifecycle_session(owner, session)
            .ok_or(PortError::NotFound)?;
        let current = record.operations().last().ok_or(PortError::Busy)?;
        if current.intent() != intent || record.phase() != SessionPhase::Running {
            return Err(PortError::Busy);
        }
        let saved = current.unloaded().ok_or(PortError::Busy)?.clone();
        let prior = saved.outcome.continued().ok_or(PortError::Busy)?;
        let origin = record
            .current_execution_origin()
            .ok_or(PortError::Busy)?
            .clone();
        let facts = if matches!(origin.request.operation, LifecycleOperation::Resume { .. }) {
            let observation = self
                .current_resume_observation_before(intent, deadline)
                .map_err(CurrentObservationError::into_port)?;
            match &observation.readiness {
                CurrentResumeReadiness::StateLost => {
                    return self.publish_continued_loss_before(&observation, &saved, deadline);
                }
                CurrentResumeReadiness::Observed { facts } => facts.clone(),
                _ => return Err(PortError::Busy),
            }
        } else {
            self.observe_continued_create_before(&origin, intent, deadline)?
        };
        let after = self.now()?;
        if after >= facts.end_at_ms
            || facts.started_at_ms != prior.started_at_ms
            || facts.end_at_ms != prior.end_at_ms
            || facts.max_end_at_ms != prior.max_end_at_ms
            || facts.startup_deadline_ms != prior.startup_deadline_ms
        {
            return Err(PortError::Busy);
        }
        let mut authority = self
            .authority
            .lock()
            .map_err(|_| PortError::OutcomeUnknown)?;
        // Reaffirm the SAME immutable historical outcome in the original CAS.
        // Fresh readiness is detached evidence, never a replacement saved Ready.
        authority
            .accept_lifecycle_unload_observation(&saved, after)
            .map_err(authority_error)?;
        let current = authority.public_lifecycle_record(owner, session)?;
        let operation = current.operations().last().ok_or(PortError::Busy)?;
        if operation.intent() != intent
            || operation.unloaded() != Some(&saved)
            || current.current_execution_origin() != Some(&origin)
            || current.phase() != SessionPhase::Running
            || self.now()? >= facts.end_at_ms
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
