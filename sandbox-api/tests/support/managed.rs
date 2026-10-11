//! Original local-controller component fixture; never a production adapter.
use sandproof_api::{
    BuildPort, BuildSettlementView, BuildWindowClosed, ConnectRequest, ControllerPort,
    CreateRequest, ListQuery, PortError, PublicationView, SandboxDetail, SandboxPage,
    SandboxSession, TemplatePage, TemplateView,
};
use sandproof_controller_service::application::{
    self as app, ControllerApplication, ControllerHost,
};
use std::sync::Arc;

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

impl<H: ControllerHost> BuildPort for ManagedPort<H> {
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
    ) -> Result<sandproof_api::BuildWindowClosed, PortError> {
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
        request: sandproof_api::BuildSettlementRequest,
    ) -> Result<sandproof_api::BuildSettlementView, PortError> {
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
        create: Option<sandproof_api::BuildVmRequest>,
    ) -> Result<sandproof_api::BuildVmView, PortError> {
        self.application
            .build_vm_in_window(tenant, id, window, create.map(Into::into))
            .map(Into::into)
    }
    fn kill_build_vm(&self, tenant: &str, sandbox_id: &str) -> Result<(), PortError> {
        self.application.kill(tenant, sandbox_id)
    }
}

impl<H: ControllerHost> ControllerPort for ManagedPort<H> {
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
        sandproof_api::page_to_sdk(self.application.list(tenant, selection.application)?, order)
    }
    fn kill(&self, tenant: &str, external: &str) -> Result<(), PortError> {
        self.application.kill(tenant, external)
    }
    fn set_timeout(&self, tenant: &str, external: &str, seconds: i64) -> Result<(), PortError> {
        self.application.set_timeout(tenant, external, seconds)
    }
}
