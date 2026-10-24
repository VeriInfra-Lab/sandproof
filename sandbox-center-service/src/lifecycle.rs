//! Current lifecycle admission and authorized observations use one authority.
//! Host I/O never holds the center's shared decision lock.
use crate::application::tenant_id;
use crate::{
    CenterApplication, CenterPort, HostPort, TemplateProvider, business::SharedBusinessAuthority,
};
use sandproof_api::{PortError, lifecycle::LifecyclePort};
use sandproof_center_core::SessionPhase;
use sandproof_protocol::lifecycle::{
    AcceptedLifecycleIntent, CurrentCreateReadiness, LifecycleOperation, PublicAcceptance,
    PublicCreateRequest, PublicDestroyRequest, PublicSessionPhase, PublicSessionView,
    PublicUnloadRequest, SessionId,
};

impl<H: HostPort, T: TemplateProvider> CenterApplication<SharedBusinessAuthority, H, T> {
    /// # Errors
    /// No caller chooses another owner or resets the persisted window frontier.
    pub fn lifecycle_request_window(
        &self,
        owner: &str,
    ) -> Result<sandproof_protocol::lifecycle::PublicRequestWindow, PortError> {
        let owner = tenant_id(owner)?;
        let current = self.snapshot()?;
        Ok(sandproof_protocol::lifecycle::PublicRequestWindow {
            window: current
                .public_request_window(&owner)
                .map_err(|error| crate::application::authority_error(error.into()))?,
        })
    }
    /// # Errors
    /// Acceptance only; cleanup still requires original physical receipts.
    pub fn destroy_lifecycle(
        &self,
        owner: &str,
        session: SessionId,
        request: &PublicDestroyRequest,
    ) -> Result<PublicAcceptance, PortError> {
        let owner = tenant_id(owner)?;
        let intent = self
            .authority
            .lock()
            .map_err(|_| PortError::OutcomeUnknown)?
            .accept_public_destroy(&owner, session, request)?;
        Ok(acceptance(intent))
    }
    /// # Errors
    /// Only the current confirmed authority can accept; host completion is separate.
    pub fn create_lifecycle(
        &self,
        owner: &str,
        request: &PublicCreateRequest,
    ) -> Result<PublicAcceptance, PortError> {
        let owner = tenant_id(owner)?;
        let intent = self
            .authority
            .lock()
            .map_err(|_| PortError::OutcomeUnknown)?
            .accept_public_create(&owner, request, (&self.config).into(), || self.now())?;
        Ok(acceptance(intent))
    }

    /// # Errors
    /// A new stale version rejects; a matching full replay returns its original acceptance.
    pub fn unload_lifecycle(
        &self,
        owner: &str,
        session: &SessionId,
        request: &PublicUnloadRequest,
    ) -> Result<PublicAcceptance, PortError> {
        let owner = tenant_id(owner)?;
        if let Some(intent) = self
            .authority
            .lock()
            .map_err(|_| PortError::OutcomeUnknown)?
            .replay_public_unload(&owner, session, request)?
        {
            return Ok(acceptance(intent));
        }
        let (current, phase) = self.current_lifecycle_intent(&owner, session)?;
        if !matches!(phase, SessionPhase::Creating | SessionPhase::Running) {
            return Err(PortError::Conflict);
        }
        let observation = self.current_create_observation(&current)?;
        let now = self.now()?;
        let intent = self
            .authority
            .lock()
            .map_err(|_| PortError::OutcomeUnknown)?
            .accept_public_unload(&owner, session, request, &observation, now)?;
        Ok(acceptance(intent))
    }

    /// # Errors
    /// This view never infers a physical fact from timeouts or an internal Paused.
    pub fn query_lifecycle(
        &self,
        owner: &str,
        session: &SessionId,
    ) -> Result<PublicSessionView, PortError> {
        let owner = tenant_id(owner)?;
        let (intent, phase) = self
            .authority
            .lock()
            .map_err(|_| PortError::OutcomeUnknown)?
            .public_lifecycle_intent(&owner, session)?;
        if phase == SessionPhase::Unloaded {
            let mut authority = self
                .authority
                .lock()
                .map_err(|_| PortError::OutcomeUnknown)?;
            crate::AuthorityPort::expire_current(&mut *authority, self.now()?)
                .map_err(crate::application::authority_error)?;
            return authority.query_public_lifecycle(&owner, session);
        }
        if matches!(
            phase,
            SessionPhase::Creating | SessionPhase::Running | SessionPhase::Renewing
        ) && matches!(
            intent.request.operation,
            LifecycleOperation::Create { .. } | LifecycleOperation::Renew { .. }
        ) {
            // A local Running can already be committed while the center still
            // retains pending Create. Observe that same operation; never issue
            // another create or infer current readiness from saved history.
            let observation = match self.current_create_observation_attempt(&intent) {
                Ok(observation) => Some(observation),
                Err(crate::lifecycle_dispatch::CurrentObservationError::Shared(error)) => {
                    return Err(error);
                }
                Err(
                    crate::lifecycle_dispatch::CurrentObservationError::Host(_)
                    | crate::lifecycle_dispatch::CurrentObservationError::Superseded
                    | crate::lifecycle_dispatch::CurrentObservationError::Deadline,
                ) => None,
            };
            if let Some(observation) = observation {
                if matches!(
                    observation.readiness,
                    CurrentCreateReadiness::Refused { .. }
                ) {
                    let mut authority = self
                        .authority
                        .lock()
                        .map_err(|_| PortError::OutcomeUnknown)?;
                    crate::AuthorityPort::accept_lifecycle_create_refusal(
                        &mut *authority,
                        &observation,
                    )
                    .map_err(crate::application::authority_error)?;
                    return authority.query_public_lifecycle(&owner, session);
                }
                let CurrentCreateReadiness::Observed { facts } = &observation.readiness else {
                    return Err(PortError::Busy);
                };
                let mut authority = self
                    .authority
                    .lock()
                    .map_err(|_| PortError::OutcomeUnknown)?;
                crate::AuthorityPort::accept_lifecycle_create_observation(
                    &mut *authority,
                    &observation,
                )
                .map_err(crate::application::authority_error)?;
                // Commit/reaffirm first, then inspect a fresh authority under
                // the same short lock. A newer unload/termination cannot be
                // overwritten by the result of the detached observation.
                let (current, phase) = authority.public_lifecycle_intent(&owner, session)?;
                if current != intent
                    || phase != SessionPhase::Running
                    || self.now()? >= facts.end_at_ms
                {
                    return Err(PortError::Busy);
                }
                return Ok(PublicSessionView {
                    session: session.clone(),
                    operation: current.request.binding.key.clone(),
                    current_version: current.accepted_version,
                    phase: PublicSessionPhase::Running,
                    unloaded: None,
                    create_refusal: None,
                });
            }
        }
        // Creating is a logical acceptance; Unloading/Terminating report only
        // the current intent. The authority refuses historical-only Running.
        self.authority
            .lock()
            .map_err(|_| PortError::OutcomeUnknown)?
            .query_public_lifecycle(&owner, session)
    }
}

impl<H: HostPort, T: TemplateProvider> LifecyclePort for CenterPort<SharedBusinessAuthority, H, T> {
    fn request_window(
        &self,
        owner: &str,
    ) -> Result<sandproof_protocol::lifecycle::PublicRequestWindow, PortError> {
        self.application().lifecycle_request_window(owner)
    }
    fn destroy_lifecycle(
        &self,
        owner: &str,
        session: SessionId,
        request: PublicDestroyRequest,
    ) -> Result<PublicAcceptance, PortError> {
        self.application()
            .destroy_lifecycle(owner, session, &request)
    }
    fn create_lifecycle(
        &self,
        owner: &str,
        request: PublicCreateRequest,
    ) -> Result<PublicAcceptance, PortError> {
        self.application().create_lifecycle(owner, &request)
    }
    fn unload_lifecycle(
        &self,
        owner: &str,
        session: SessionId,
        request: PublicUnloadRequest,
    ) -> Result<PublicAcceptance, PortError> {
        self.application()
            .unload_lifecycle(owner, &session, &request)
    }
    fn query_lifecycle(
        &self,
        owner: &str,
        session: SessionId,
    ) -> Result<PublicSessionView, PortError> {
        self.application().query_lifecycle(owner, &session)
    }
}

fn acceptance(intent: AcceptedLifecycleIntent) -> PublicAcceptance {
    PublicAcceptance {
        session: intent.request.binding.session,
        key: intent.request.binding.key,
        accepted_version: intent.accepted_version,
    }
}
