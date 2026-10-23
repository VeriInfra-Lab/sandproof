//! Private caller authentication is supplied by the protected API assembly.
//! Every authorized action still consults the SAME fresh business envelope;
//! no private token cache or second lifecycle/controller is introduced.
use crate::business::{BuildParameters, GlobalBuildAnchor, SharedBusinessAuthority};
use crate::{CenterApplication, CommittedDriver, DataRoute, HostPort, TemplateProvider};
use sandproof_api::{
    BuildSettlementRequest, BuildSettlementView, BuildVmRequest, BuildVmView, BuildWindowClosed,
    PortError,
};
use sandproof_center_core::Sandbox;
use sandproof_center_store::business::BusinessEnvelope;
use sandproof_controller_service::{
    BuildWindow, CreateNetworking, CreateOptions,
    application::{BuildStatus, Session},
};
use sandproof_protocol::{SandboxId, TenantId, lifecycle::LifecycleOperation};
use std::sync::Arc;

pub struct CenterBuildPort<H: HostPort, T: TemplateProvider>(
    Arc<CenterApplication<SharedBusinessAuthority, H, T>>,
);
impl<H: HostPort, T: TemplateProvider> CenterBuildPort<H, T> {
    #[must_use]
    pub fn new(app: Arc<CenterApplication<SharedBusinessAuthority, H, T>>) -> Self {
        Self(app)
    }
    #[must_use]
    pub fn application(&self) -> &Arc<CenterApplication<SharedBusinessAuthority, H, T>> {
        &self.0
    }
}
type Envelope = BusinessEnvelope<crate::business::RegistryMetadata>;
fn anchored<'a>(state: &'a Envelope, anchor: &GlobalBuildAnchor) -> Result<&'a Sandbox, PortError> {
    state
        .center
        .sandboxes()
        .iter()
        .find(|record| {
            record
                .operations
                .first()
                .is_some_and(|first| first.intent() == &anchor.original_create)
        })
        .ok_or(PortError::NotFound)
}
impl<H: HostPort, T: TemplateProvider> CenterApplication<SharedBusinessAuthority, H, T> {
    fn business(&self) -> Result<SharedBusinessAuthority, PortError> {
        self.authority
            .lock()
            .map(|authority| authority.clone())
            .map_err(|_| PortError::OutcomeUnknown)
    }
    fn builder_status(
        &self,
        tenant: &TenantId,
        window: &str,
        id: &str,
    ) -> Result<BuildVmView, PortError> {
        let business = self.business()?;
        let (state, anchor) = business.snapshot_build_anchor(tenant, window, id)?;
        let anchor = anchor.ok_or(PortError::NotFound)?;
        let record = anchored(&state, &anchor)?;
        let now = self.now()?;
        let session = if crate::projection::live(record, now) {
            if !state
                .center
                .node_is_reachable(&anchor.original_create.request.binding.node, now)
            {
                return Err(PortError::Busy);
            }
            let LifecycleOperation::Create { create, .. } =
                &anchor.original_create.request.operation
            else {
                return Err(PortError::OutcomeUnknown);
            };
            let template = state
                .metadata
                .bound(&create.template_revision, &create.template_digest)?;
            let detail = self.detail_for_record(&state.center, tenant, record, &template)?;
            Some(Session {
                template_id: detail.info.template_id,
                sandbox_id: detail.info.sandbox_id,
                owner: detail.info.owner,
                envd_version: detail.info.envd_version,
                envd_access_token: detail.envd_access_token,
                domain: detail.domain,
            })
        } else {
            None
        };
        let state = if record.released() {
            "Destroyed"
        } else if record.terminating() {
            "Revoking"
        } else if record.last_running().is_some() {
            "Running"
        } else {
            "Starting"
        };
        Ok(BuildStatus {
            sandbox_id: anchor
                .original_create
                .request
                .binding
                .session
                .as_protocol_id()
                .as_str()
                .to_owned(),
            state: state.into(),
            cleanup_complete: record.released(),
            session,
        }
        .into())
    }
    /// Only the already authenticated private builder may call this method.
    /// # Errors
    /// An exact retry tuple is mandatory; unknown accepted work retains it.
    pub fn private_build_vm(
        &self,
        tenant: &str,
        id: &str,
        window: Option<&str>,
        create: Option<BuildVmRequest>,
    ) -> Result<BuildVmView, PortError> {
        let tenant = crate::application::tenant_id(tenant)?;
        let window = window.ok_or(PortError::Conflict)?;
        let Some(input) = create else {
            return self.builder_status(&tenant, window, id);
        };
        if input.request_window.as_deref() != Some(window) {
            return Err(PortError::Invalid);
        }
        let business = self.business()?;
        let initial = business.snapshot()?;
        let private = initial
            .metadata
            .private_build_config()
            .ok_or(PortError::NotFound)?;
        if input.allow_internet_access && !private.policy.public_ipv4_enabled {
            return Err(PortError::Invalid);
        }
        let template = self
            .templates
            .bound(&private.builder_revision, &private.builder_digest)?;
        let now = self.now()?;
        let binding = crate::create_binding::create_parameters(
            &template,
            &CreateOptions {
                networking: Some(CreateNetworking {
                    allow_internet_access: Some(input.allow_internet_access),
                    network: None,
                }),
                ..CreateOptions::default()
            },
            input.timeout,
            private.policy,
        )?;
        let parameters = BuildParameters {
            timeout_secs: input.timeout,
            allow_internet_access: input.allow_internet_access,
        };
        let request = business.accept_build(&tenant, window, id, &parameters, &binding, now)?;
        let current = business.snapshot().map_err(|_| PortError::OutcomeUnknown)?;
        let record = current
            .center
            .sandboxes()
            .iter()
            .find(|record| {
                record
                    .operations
                    .first()
                    .is_some_and(|first| first.intent() == &request)
            })
            .ok_or(PortError::OutcomeUnknown)?;
        // A terminal retry must query its original tombstone, not dispatch the
        // superseded create or construct another sandbox.
        if record
            .operations
            .last()
            .is_some_and(|last| last.intent() == &request)
        {
            self.wait_current_running(&request)?;
        }
        self.builder_status(&tenant, window, id)
            .map_err(|_| PortError::OutcomeUnknown)
    }
    /// # Errors
    /// Registered tenant, original monotone windows and same CAS are required.
    pub fn private_issue_build_window(&self, tenant: &str) -> Result<BuildWindow, PortError> {
        self.business()?
            .issue_build_window(&crate::application::tenant_id(tenant)?, self.now()?)
    }
    /// # Errors
    /// Acknowledgement comes only after the original admission fence is durable.
    pub fn private_close_build_window(
        &self,
        tenant: &str,
        window: &str,
    ) -> Result<BuildWindowClosed, PortError> {
        self.business()?.close_build_window(
            &crate::application::tenant_id(tenant)?,
            window,
            self.now()?,
        )?;
        Ok(BuildWindowClosed {
            request_window: window.into(),
            closed: true,
        })
    }
    /// # Errors
    /// Exact closed window and original Cleaned evidence precede anchor release.
    pub fn private_settle_build(
        &self,
        tenant: &str,
        id: &str,
        input: BuildSettlementRequest,
    ) -> Result<BuildSettlementView, PortError> {
        let sandbox = input
            .sandbox_id
            .as_deref()
            .map(|id| SandboxId::try_from(id).map_err(|_| PortError::Invalid))
            .transpose()?;
        self.business()?.settle_build(
            &crate::application::tenant_id(tenant)?,
            &input.request_window,
            id,
            sandbox.as_ref(),
        )?;
        Ok(BuildSettlementView {
            request_window: input.request_window,
            settled: true,
        })
    }
    /// # Errors
    /// Private management cannot reach a public or validation sandbox, even
    /// belonging to the same tenant; cleanup still uses the original driver.
    pub fn private_kill(&self, tenant: &str, id: &str) -> Result<(), PortError> {
        let tenant = crate::application::tenant_id(tenant)?;
        let sandbox: SandboxId = id.try_into().map_err(|_| PortError::NotFound)?;
        let state = self.business()?.snapshot()?;
        let anchor = state
            .metadata
            .private_build_anchors()
            .iter()
            .find(|anchor| {
                anchor.tenant == tenant
                    && anchor
                        .original_create
                        .request
                        .binding
                        .session
                        .as_protocol_id()
                        == &sandbox
            })
            .ok_or(PortError::NotFound)?;
        anchored(&state, anchor)?;
        let request = self.destroy_exact(&anchor.original_create)?;
        self.wait_current_terminal(&request)
    }
    /// Caller must have authenticated the private backend credential FIRST.
    /// # Errors
    /// Token, job anchor, owner, original lifetime and fresh node all agree.
    pub fn resolve_private_data_route(
        &self,
        tenant: &str,
        token: &str,
    ) -> Result<DataRoute, PortError> {
        let tenant = crate::application::tenant_id(tenant)?;
        let state = self.business()?.snapshot()?;
        let now = self.now()?;
        let mut result = None;
        for anchor in state.metadata.private_build_anchors() {
            if anchor.tenant != tenant {
                continue;
            }
            let record = anchored(&state, anchor)?;
            let route = match Self::route_for_record(&state.center, record, now) {
                Ok(route) => route,
                Err(PortError::NotFound) => continue,
                Err(error) => return Err(error),
            };
            let matching = self.tokens.verify(&route.identity, token);
            if matching {
                if !state
                    .center
                    .node_is_reachable(&anchor.original_create.request.binding.node, now)
                {
                    sandproof_controller_service::application::fault::FaultRecord::application(
                        sandproof_controller_service::application::fault::FaultStage::CenterNodeFreshness,
                        &PortError::Busy,
                        sandproof_controller_service::application::fault::FaultContext::default(),
                    ).emit();
                    return Err(PortError::Busy);
                }
                if result.is_some() {
                    return Err(PortError::Busy);
                }
                result = Some(route);
            }
        }
        result.ok_or(PortError::NotFound)
    }
}

impl<H: HostPort, T: TemplateProvider> sandproof_api::BuildPort for CenterBuildPort<H, T> {
    fn build_vm_in_window(
        &self,
        tenant: &str,
        id: &str,
        window: Option<&str>,
        create: Option<BuildVmRequest>,
    ) -> Result<BuildVmView, PortError> {
        self.0.private_build_vm(tenant, id, window, create)
    }
    fn issue_build_window(&self, tenant: &str) -> Result<BuildWindow, PortError> {
        self.0.private_issue_build_window(tenant)
    }
    fn close_build_window(
        &self,
        tenant: &str,
        window: &str,
    ) -> Result<BuildWindowClosed, PortError> {
        self.0.private_close_build_window(tenant, window)
    }
    fn settle_build_vm(
        &self,
        tenant: &str,
        id: &str,
        request: BuildSettlementRequest,
    ) -> Result<BuildSettlementView, PortError> {
        self.0.private_settle_build(tenant, id, request)
    }
    fn kill_build_vm(&self, tenant: &str, sandbox_id: &str) -> Result<(), PortError> {
        self.0.private_kill(tenant, sandbox_id)
    }
}

impl<H: HostPort, T: TemplateProvider> sandproof_api::BuildPublicationPort
    for CenterBuildPort<H, T>
{
    fn publish_build_candidate(
        &self,
        tenant: &str,
        candidate: &str,
    ) -> Result<sandproof_api::PublicationView, PortError> {
        self.0.templates.publish(self.0.as_ref(), tenant, candidate)
    }
    fn build_publication(
        &self,
        tenant: &str,
        candidate: &str,
    ) -> Result<sandproof_api::PublicationView, PortError> {
        self.0.templates.publication(tenant, candidate)
    }
}
