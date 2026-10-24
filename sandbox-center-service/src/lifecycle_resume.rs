//! Detached observations of the same persisted resumed execution.
use crate::application::{authority_error, diagnostics};
use crate::lifecycle_dispatch::{CurrentObservationError, time_left};
use crate::{AuthorityPort, CenterApplication, HostPort, TemplateProvider};
use sandproof_api::PortError;
use sandproof_center_core::{DecisionError, SessionPhase};
use sandproof_center_store::authority::AuthorityError;
use sandproof_protocol::lifecycle::{
    AcceptedLifecycleIntent, CurrentResumeReadiness, LifecycleOperation, ResumeObservation,
};
use std::time::Instant;

impl<A: AuthorityPort, H: HostPort, T: TemplateProvider> CenterApplication<A, H, T> {
    pub(crate) fn is_resumed_execution(
        &self,
        intent: &AcceptedLifecycleIntent,
    ) -> Result<bool, PortError> {
        if matches!(intent.request.operation, LifecycleOperation::Resume { .. }) {
            return Ok(true);
        }
        if !matches!(intent.request.operation, LifecycleOperation::Renew { .. }) {
            return Ok(false);
        }
        let state = self.snapshot()?;
        let record = state
            .lifecycle_session(
                &intent.request.binding.key.owner,
                &intent.request.binding.session,
            )
            .ok_or(PortError::NotFound)?;
        Ok(record.current_execution_origin().is_some_and(|origin| {
            matches!(origin.request.operation, LifecycleOperation::Resume { .. })
        }))
    }

    pub(crate) fn current_resume_observation_before(
        &self,
        intent: &AcceptedLifecycleIntent,
        deadline: Instant,
    ) -> Result<ResumeObservation, CurrentObservationError> {
        let policy = self
            .lifecycle_observation_policy
            .ok_or(CurrentObservationError::Shared(PortError::Busy))?;
        let state = self.snapshot().map_err(CurrentObservationError::Shared)?;
        let record = state
            .lifecycle_session(
                &intent.request.binding.key.owner,
                &intent.request.binding.session,
            )
            .ok_or(CurrentObservationError::Superseded)?;
        if record
            .operations()
            .last()
            .is_none_or(|value| value.intent() != intent)
        {
            return Err(CurrentObservationError::Superseded);
        }
        let origin = record
            .current_execution_origin()
            .ok_or(CurrentObservationError::Superseded)?;
        if !matches!(origin.request.operation, LifecycleOperation::Resume { .. }) {
            return Err(CurrentObservationError::Superseded);
        }
        let before = self.now().map_err(CurrentObservationError::Shared)?;
        let budget = time_left(deadline).map_err(|_| CurrentObservationError::Deadline)?;
        let view = self
            .hosts
            .observe_lifecycle_resume(origin, budget)
            .map_err(CurrentObservationError::Host)?;
        let after = self.now().map_err(CurrentObservationError::Shared)?;
        if after < before {
            return Err(CurrentObservationError::Shared(PortError::OutcomeUnknown));
        }
        time_left(deadline).map_err(|_| CurrentObservationError::Deadline)?;
        if !view.matches_request(origin) || view.current_request != *intent {
            return Err(CurrentObservationError::Superseded);
        }
        match &view.readiness {
            CurrentResumeReadiness::Observed { facts } => {
                if policy
                    .healthy_until(before, after, facts.observed_at_ms)
                    .is_none()
                    || after >= facts.end_at_ms
                {
                    return Err(CurrentObservationError::Host(PortError::Busy));
                }
            }
            CurrentResumeReadiness::StateLost => {}
            _ => return Err(CurrentObservationError::Host(PortError::Busy)),
        }
        Ok(view)
    }

    pub(crate) fn reconcile_resume(
        &self,
        intent: &AcceptedLifecycleIntent,
        deadline: Instant,
    ) -> Result<bool, PortError> {
        let observation = match self.current_resume_observation_before(intent, deadline) {
            Ok(value) => value,
            Err(CurrentObservationError::Shared(error)) => return Err(error),
            Err(_) => return Ok(false),
        };
        let result = self
            .authority
            .lock()
            .map_err(|_| PortError::OutcomeUnknown)?
            .accept_lifecycle_resume_observation(&observation);
        match result {
            Ok(()) => match self.confirm_pending_sdk_renewal_before(
                &intent.request.binding.key.owner,
                &intent.request.binding.session,
                deadline,
            ) {
                Err(CurrentObservationError::Shared(error)) => Err(error),
                Ok(_) | Err(_) => Ok(true),
            },
            Err(AuthorityError::Decision(
                DecisionError::Stale
                | DecisionError::Conflict
                | DecisionError::Unavailable
                | DecisionError::NotFound,
            )) => Ok(false),
            Err(error) => Err(authority_error(error)),
        }
    }
}

impl<H: HostPort, T: TemplateProvider>
    CenterApplication<crate::business::SharedBusinessAuthority, H, T>
{
    pub(crate) fn query_resumed(
        &self,
        owner: &sandproof_protocol::TenantId,
        session: &sandproof_protocol::lifecycle::SessionId,
        intent: &AcceptedLifecycleIntent,
    ) -> Result<sandproof_protocol::lifecycle::PublicSessionView, PortError> {
        let deadline = self.reply_deadline()?;
        let observation = match self.current_resume_observation_before(intent, deadline) {
            Ok(view) => Some(view),
            Err(CurrentObservationError::Shared(error)) => return Err(error),
            Err(_) => None,
        };
        let mut authority = self
            .authority
            .lock()
            .map_err(|_| PortError::OutcomeUnknown)?;
        if let Some(observation) = observation {
            authority
                .accept_lifecycle_resume_observation(&observation)
                .map_err(authority_error)?;
            let (current, phase) = authority.public_lifecycle_intent(owner, session)?;
            if let CurrentResumeReadiness::Observed { facts } = observation.readiness {
                if current != *intent
                    || phase != SessionPhase::Running
                    || self.now()? >= facts.end_at_ms
                {
                    return Err(PortError::Busy);
                }
                return Ok(sandproof_protocol::lifecycle::PublicSessionView {
                    session: session.clone(),
                    operation: current.request.binding.key,
                    current_version: current.accepted_version,
                    phase: sandproof_protocol::lifecycle::PublicSessionPhase::Running,
                    unload_outcome: None,
                    create_refusal: None,
                });
            }
        }
        authority
            .query_public_lifecycle(owner, session)
            .inspect_err(|error| {
                diagnostics::emit(
                    diagnostics::Stage::LifecycleQueryResult,
                    diagnostics::Failure::Port(*error),
                );
            })
    }
}
