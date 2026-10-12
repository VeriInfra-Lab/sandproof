//! E2B wire adapter only. SDK DTOs, HTTP parsing, error mapping and list cursor
//! serialization stay in sandproof-api; global business decisions live here.
use crate::{AuthorityPort, CenterApplication, HostPort, TemplateProvider};
use sandproof_api::{
    ConnectRequest, ControllerPort, CreateRequest, ListQuery, PortError, PublicationView,
    SandboxDetail, SandboxPage, SandboxSession, TemplatePage, TemplateView,
};
use std::sync::Arc;

pub struct CenterPort<A: AuthorityPort, H: HostPort, T: TemplateProvider>(
    Arc<CenterApplication<A, H, T>>,
);
impl<A: AuthorityPort, H: HostPort, T: TemplateProvider> CenterPort<A, H, T> {
    #[must_use]
    pub fn new(application: Arc<CenterApplication<A, H, T>>) -> Self {
        Self(application)
    }
    #[must_use]
    pub fn application(&self) -> &Arc<CenterApplication<A, H, T>> {
        &self.0
    }
}
impl<A: AuthorityPort, H: HostPort, T: TemplateProvider> ControllerPort for CenterPort<A, H, T> {
    fn create(&self, tenant: &str, request: CreateRequest) -> Result<SandboxSession, PortError> {
        self.0.create(tenant, request.into()).map(Into::into)
    }
    fn connect(
        &self,
        tenant: &str,
        sandbox: &str,
        request: ConnectRequest,
    ) -> Result<SandboxSession, PortError> {
        self.0
            .connect(
                tenant,
                sandbox,
                sandproof_controller_service::application::ConnectCommand {
                    timeout: request.timeout,
                },
            )
            .map(Into::into)
    }
    fn get(&self, tenant: &str, sandbox: &str) -> Result<SandboxDetail, PortError> {
        self.0.get(tenant, sandbox)?.try_into()
    }
    fn list(&self, tenant: &str, query: ListQuery) -> Result<SandboxPage, PortError> {
        let selection = query.validate()?;
        sandproof_api::page_to_sdk(self.0.list(tenant, selection.application)?, selection.order)
    }
    fn kill(&self, tenant: &str, sandbox: &str) -> Result<(), PortError> {
        self.0.kill(tenant, sandbox)
    }
    fn set_timeout(&self, tenant: &str, sandbox: &str, seconds: i64) -> Result<(), PortError> {
        self.0.set_timeout(tenant, sandbox, seconds)
    }
    fn publish_template(
        &self,
        tenant: &str,
        candidate: &str,
    ) -> Result<PublicationView, PortError> {
        self.0.templates.publish(self.0.as_ref(), tenant, candidate)
    }
    fn publication(&self, tenant: &str, candidate: &str) -> Result<PublicationView, PortError> {
        self.0.templates.publication(tenant, candidate)
    }
    fn template(&self, tenant: &str, name: &str) -> Result<TemplateView, PortError> {
        self.0.templates.get(tenant, name)
    }
    fn templates(
        &self,
        tenant: &str,
        limit: usize,
        cursor: Option<&str>,
    ) -> Result<TemplatePage, PortError> {
        self.0.templates.list(tenant, limit, cursor)
    }
    fn set_template_alias(
        &self,
        tenant: &str,
        name: &str,
        target: &str,
        expected: Option<&str>,
    ) -> Result<TemplateView, PortError> {
        self.0.templates.set_alias(tenant, name, target, expected)
    }
    fn delete_template_alias(&self, tenant: &str, name: &str) -> Result<(), PortError> {
        self.0.templates.delete_alias(tenant, name)
    }
}
