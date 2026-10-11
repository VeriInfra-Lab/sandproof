//! Thin borrowed template workflow driver, not another lifecycle algorithm.
use crate::{
    AuthorityPort, CenterApplication, CenterConfig, CommittedDriver, HostPort, TemplateProvider,
};
use sandproof_api::PortError;
use sandproof_center_core::CenterState;
use sandproof_protocol::{
    SandboxId, TenantId,
    internal::lifecycle::{LifecycleRequest, LifecycleResponse},
};

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
    fn drive_committed(&self, request: &LifecycleRequest) -> Result<LifecycleResponse, PortError> {
        self.wait(request)
    }
    fn destroy_exact(
        &self,
        tenant: &TenantId,
        sandbox: &SandboxId,
    ) -> Result<LifecycleRequest, PortError> {
        self.authority
            .lock()
            .map_err(|_| PortError::OutcomeUnknown)?
            .destroy(tenant, sandbox)
            .map_err(crate::application::authority_error)
    }
}
