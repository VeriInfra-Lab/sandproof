//! The original application and original business authority own v3 admission.
//! No host call is made while accepting or querying a durable operation.
use crate::application::tenant_id;
use crate::{
    CenterApplication, CenterPort, HostPort, TemplateProvider, business::SharedBusinessAuthority,
};
use sandproof_api::{PortError, lifecycle::LifecyclePort};
use sandproof_protocol::lifecycle::{
    AcceptedLifecycleIntent, PublicAcceptance, PublicCreateRequest, PublicSessionView,
    PublicUnloadRequest, SessionId,
};

impl<H: HostPort, T: TemplateProvider> CenterApplication<SharedBusinessAuthority, H, T> {
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
        session: SessionId,
        request: &PublicUnloadRequest,
    ) -> Result<PublicAcceptance, PortError> {
        let owner = tenant_id(owner)?;
        let intent = self
            .authority
            .lock()
            .map_err(|_| PortError::OutcomeUnknown)?
            .accept_public_unload(&owner, session, request)?;
        Ok(acceptance(intent))
    }

    /// # Errors
    /// This view never infers a physical fact from timeouts or an internal Paused.
    pub fn query_lifecycle(
        &self,
        owner: &str,
        session: &SessionId,
    ) -> Result<PublicSessionView, PortError> {
        self.authority
            .lock()
            .map_err(|_| PortError::OutcomeUnknown)?
            .query_public_lifecycle(&tenant_id(owner)?, session)
    }
}

impl<H: HostPort, T: TemplateProvider> LifecyclePort for CenterPort<SharedBusinessAuthority, H, T> {
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
            .unload_lifecycle(owner, session, &request)
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
