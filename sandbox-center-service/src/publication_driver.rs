//! Thin borrowed template workflow driver, not another lifecycle algorithm.
use crate::{
    AuthorityPort, CenterApplication, CenterConfig, CommittedDriver, HostPort, TemplateProvider,
};
use sandproof_api::PortError;
use sandproof_center_core::CenterState;
use sandproof_protocol::lifecycle::AcceptedLifecycleIntent;

impl<A: AuthorityPort, H: HostPort, T: TemplateProvider> CommittedDriver
    for CenterApplication<A, H, T>
{
    fn now_ms(&self) -> Result<u64, PortError> {
        self.now()
    }
    fn validation_config(&self) -> &CenterConfig {
        &self.config
    }
    fn snapshot(&self) -> Result<CenterState, PortError> {
        Self::snapshot(self)
    }
    fn drive_committed(&self, request: &AcceptedLifecycleIntent) -> Result<(), PortError> {
        self.reconcile_lifecycle_intent(request, self.reply_deadline()?)
            .map(|_| ())
    }
    fn destroy_exact(
        &self,
        original_create: &AcceptedLifecycleIntent,
    ) -> Result<AcceptedLifecycleIntent, PortError> {
        self.authority
            .lock()
            .map_err(|_| PortError::OutcomeUnknown)?
            .destroy_bound_create(original_create)
            .map_err(crate::application::authority_error)
    }
}
