//! Thin SDK adapter. Business orchestration lives in controller-service.
use crate::{
    BuildSettlementView, BuildVmRequest, BuildVmView, BuildWindowClosed, ConnectRequest,
    ControllerPort, CreateRequest, ListQuery, PortError, PublicationView, SandboxDetail,
    SandboxInfo, SandboxLifecycle, SandboxOnTimeout, SandboxPage, SandboxSession, TemplatePage,
    TemplateView,
};
use sandproof_controller_service::application::{
    self as app, ControllerApplication, ControllerHost,
};
use std::sync::Arc;
use time::{OffsetDateTime, format_description::well_known::Rfc3339};

pub struct ManagedPort<H: ControllerHost> {
    application: Arc<ControllerApplication<H>>,
}
impl<H: ControllerHost> ManagedPort<H> {
    #[must_use]
    pub fn new(application: Arc<ControllerApplication<H>>) -> Self {
        Self { application }
    }
    #[must_use]
    pub fn application(&self) -> &Arc<ControllerApplication<H>> {
        &self.application
    }
}

impl From<CreateRequest> for app::CreateCommand {
    fn from(value: CreateRequest) -> Self {
        Self {
            template_id: value.template_id,
            timeout: value.timeout,
            allow_internet_access: value.allow_internet_access,
            network: value.network,
            metadata: value.metadata,
            env_vars: value.env_vars,
        }
    }
}
impl From<BuildVmRequest> for app::BuildCommand {
    fn from(value: BuildVmRequest) -> Self {
        Self {
            request_window: value.request_window,
            timeout: value.timeout,
            allow_internet_access: value.allow_internet_access,
        }
    }
}
impl From<app::Session> for SandboxSession {
    fn from(value: app::Session) -> Self {
        Self {
            template_id: value.template_id,
            sandbox_id: value.sandbox_id,
            client_id: format!("tenant-{}", value.owner),
            envd_version: value.envd_version,
            envd_access_token: value.envd_access_token,
            domain: value.domain,
        }
    }
}
fn rfc3339(milliseconds: i64) -> Result<String, PortError> {
    OffsetDateTime::from_unix_timestamp_nanos(i128::from(milliseconds) * 1_000_000)
        .map_err(|_| PortError::Busy)?
        .format(&Rfc3339)
        .map_err(|_| PortError::Busy)
}
impl TryFrom<app::InstanceInfo> for SandboxInfo {
    type Error = PortError;
    fn try_from(value: app::InstanceInfo) -> Result<Self, PortError> {
        Ok(Self {
            template_id: value.template_id,
            sandbox_id: value.sandbox_id,
            client_id: format!("tenant-{}", value.owner),
            metadata: value.metadata,
            started_at: rfc3339(value.started_at_ms)?,
            end_at: rfc3339(value.end_at_ms)?,
            envd_version: value.envd_version,
            cpu_count: value.cpu_count,
            memory_mb: value.memory_mb,
            disk_size_mb: value.disk_size_mb,
            state: "running".into(),
            volume_mounts: Vec::new(),
        })
    }
}
impl TryFrom<app::InstanceDetail> for SandboxDetail {
    type Error = PortError;
    fn try_from(value: app::InstanceDetail) -> Result<Self, PortError> {
        Ok(Self {
            info: value.info.try_into()?,
            envd_access_token: value.envd_access_token,
            domain: value.domain,
            allow_internet_access: value.allow_internet_access,
            network: value.network,
            lifecycle: SandboxLifecycle {
                on_timeout: SandboxOnTimeout::Kill,
                auto_resume: false,
            },
        })
    }
}
impl From<app::BuildStatus> for BuildVmView {
    fn from(value: app::BuildStatus) -> Self {
        Self {
            sandbox_id: value.sandbox_id,
            state: value.state,
            cleanup_complete: value.cleanup_complete,
            session: value.session.map(Into::into),
        }
    }
}
impl<H: ControllerHost> ControllerPort for ManagedPort<H> {
    fn build_vm(
        &self,
        tenant: &str,
        id: &str,
        create: Option<crate::BuildVmRequest>,
    ) -> Result<crate::BuildVmView, PortError> {
        self.application
            .build_vm(tenant, id, create.map(Into::into))
            .map(Into::into)
    }
    fn issue_build_window(
        &self,
        tenant: &str,
    ) -> Result<sandproof_controller_service::BuildWindow, PortError> {
        self.application.issue_build_window(tenant)
    }
    fn close_build_window(
        &self,
        tenant: &str,
        window: &str,
    ) -> Result<crate::BuildWindowClosed, PortError> {
        self.application
            .close_build_window(tenant, window)
            .map(|v| BuildWindowClosed {
                request_window: v.request_window,
                closed: v.closed,
            })
    }
    fn settle_build_vm(
        &self,
        tenant: &str,
        id: &str,
        request: crate::BuildSettlementRequest,
    ) -> Result<crate::BuildSettlementView, PortError> {
        self.application
            .settle_build_vm(
                tenant,
                id,
                app::BuildSettlementCommand {
                    request_window: request.request_window,
                    sandbox_id: request.sandbox_id,
                },
            )
            .map(|v| BuildSettlementView {
                request_window: v.request_window,
                settled: v.settled,
            })
    }
    fn build_vm_in_window(
        &self,
        tenant: &str,
        id: &str,
        window: Option<&str>,
        create: Option<crate::BuildVmRequest>,
    ) -> Result<crate::BuildVmView, PortError> {
        self.application
            .build_vm_in_window(tenant, id, window, create.map(Into::into))
            .map(Into::into)
    }
    fn publish_template(
        &self,
        tenant: &str,
        candidate: &str,
    ) -> Result<PublicationView, PortError> {
        self.application.publish_template(tenant, candidate)
    }
    fn publication(&self, tenant: &str, candidate: &str) -> Result<PublicationView, PortError> {
        self.application.publication(tenant, candidate)
    }
    fn template(&self, tenant: &str, name: &str) -> Result<TemplateView, PortError> {
        self.application.template(tenant, name)
    }
    fn templates(
        &self,
        tenant: &str,
        limit: usize,
        cursor: Option<&str>,
    ) -> Result<TemplatePage, PortError> {
        self.application.templates(tenant, limit, cursor)
    }
    fn set_template_alias(
        &self,
        tenant: &str,
        name: &str,
        target: &str,
        expected: Option<&str>,
    ) -> Result<TemplateView, PortError> {
        self.application
            .set_template_alias(tenant, name, target, expected)
    }
    fn delete_template_alias(&self, tenant: &str, name: &str) -> Result<(), PortError> {
        self.application.delete_template_alias(tenant, name)
    }
    fn create(&self, tenant: &str, request: CreateRequest) -> Result<SandboxSession, PortError> {
        self.application
            .create(tenant, request.into())
            .map(Into::into)
    }
    fn connect(
        &self,
        tenant: &str,
        external: &str,
        request: ConnectRequest,
    ) -> Result<SandboxSession, PortError> {
        self.application
            .connect(
                tenant,
                external,
                app::ConnectCommand {
                    timeout: request.timeout,
                },
            )
            .map(Into::into)
    }
    fn get(&self, tenant: &str, external: &str) -> Result<SandboxDetail, PortError> {
        self.application.get(tenant, external)?.try_into()
    }
    fn list(&self, tenant: &str, query: ListQuery) -> Result<SandboxPage, PortError> {
        let selection = query.validate()?;
        let order = selection.order;
        crate::list::page_to_sdk(self.application.list(tenant, selection.application)?, order)
    }
    fn kill(&self, tenant: &str, external: &str) -> Result<(), PortError> {
        self.application.kill(tenant, external)
    }
    fn set_timeout(&self, tenant: &str, external: &str, seconds: i64) -> Result<(), PortError> {
        self.application.set_timeout(tenant, external, seconds)
    }
}
