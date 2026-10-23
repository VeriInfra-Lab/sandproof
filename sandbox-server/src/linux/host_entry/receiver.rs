//! Mandatory production ports share one original facade and trusted cache.
use super::cache::TemplateCache;
use axum::{
    body::Body,
    http::{Request, Response},
};
use sandproof_controller_service::templates::ComputeLimits;
use sandproof_host_service::HostFacade;
use sandproof_internal::{
    CacheReceiver, DataReceiver, LifecycleReceiver, ReceiveError, ValidationReceiver,
    cache::{CacheAvailability, CacheRequest},
    status::{HostStatus, StatusReceiver, StatusRequest},
    template::{ValidationEvidence, ValidationRequest},
};
use sandproof_node::NodeHost;
use sandproof_protocol::{
    internal::NodeIdentity,
    lifecycle::{AcceptedLifecycleIntent, AuthenticatedCenterContext, CreateObservation, LifecycleAccessIdentity, RecordedLifecycleIntent},
};
use std::sync::Arc;

pub(super) struct RegisteredHost {
    pub(super) facade: HostFacade<NodeHost>,
    pub(super) templates: Arc<TemplateCache>,
    pub(super) compute: ComputeLimits,
    pub(super) node: NodeIdentity,
    pub(super) retention_slots: u16,
}
impl LifecycleReceiver for RegisteredHost {
    async fn confirm_renewal(
        &self,
        result: sandproof_protocol::lifecycle::RenewalConfirmation,
        context: AuthenticatedCenterContext,
    ) -> Result<sandproof_protocol::lifecycle::ConfirmedRenewal, ReceiveError> {
        self.facade.confirm_renewal(result, context).await
    }
    async fn confirm_terminal(&self, result: sandproof_protocol::lifecycle::TerminalLifecycleObservation, context: AuthenticatedCenterContext) -> Result<sandproof_protocol::lifecycle::ConfirmedLifecycleResult, ReceiveError> {
        self.facade.confirm_terminal(result, context).await
    }
    async fn close_window(&self, request: sandproof_protocol::lifecycle::LifecycleWindowClose, context: AuthenticatedCenterContext) -> Result<sandproof_protocol::lifecycle::ClosedLifecycleWindow, ReceiveError> {
        self.facade.close_window(request, context).await
    }
    async fn observe_terminal(&self, intent: AcceptedLifecycleIntent, context: AuthenticatedCenterContext) -> Result<sandproof_protocol::lifecycle::TerminalLifecycleObservation, ReceiveError> {
        self.facade.observe_terminal(intent, context).await
    }
    async fn record_lifecycle(&self, intent: AcceptedLifecycleIntent, context: AuthenticatedCenterContext) -> Result<RecordedLifecycleIntent, ReceiveError> {
        self.facade.record_lifecycle(intent, context).await
    }
    async fn observe_create(&self, intent: AcceptedLifecycleIntent, context: AuthenticatedCenterContext) -> Result<CreateObservation, ReceiveError> {
        self.facade.observe_create(intent, context).await
    }
}
impl DataReceiver for RegisteredHost {
    async fn receive_data(
        &self,
        identity: LifecycleAccessIdentity,
        context: AuthenticatedCenterContext,
        request: Request<Body>,
    ) -> Response<Body> {
        self.facade.receive_data(identity, context, request).await
    }
}
impl ValidationReceiver for RegisteredHost {
    async fn validation_evidence(
        &self,
        request: ValidationRequest,
        context: AuthenticatedCenterContext,
    ) -> Result<ValidationEvidence, ReceiveError> {
        self.facade.validation_evidence(request, context).await
    }
}
impl CacheReceiver for RegisteredHost {
    async fn cache_template(
        &self,
        request: CacheRequest,
    ) -> Result<CacheAvailability, ReceiveError> {
        // Cancellation of an HTTP caller does not release a live download's
        // permit or its exact temporary responsibility.
        Arc::clone(&self.templates).receive(request).await
    }
}
impl StatusReceiver for RegisteredHost {
    async fn status(&self, request: StatusRequest) -> Result<HostStatus, ReceiveError> {
        request
            .validate(&self.node)
            .map_err(|_| ReceiveError::Conflict)?;
        let (observed_at_ms, free) = self
            .facade
            .lifecycle()
            .free_resources(self.node.clone(), self.compute)
            .await
            .map_err(|_| ReceiveError::Unavailable)?;
        Ok(HostStatus {
            node: self.node.clone(),
            retention_slots: self.retention_slots,
            observed_at_ms,
            free,
        })
    }
}
