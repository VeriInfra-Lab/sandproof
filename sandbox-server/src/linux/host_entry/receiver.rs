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
use sandproof_protocol::internal::{
    ControlIdentity, NodeIdentity,
    lifecycle::{LifecycleRequest, LifecycleResponse},
    window::{WindowCloseAck, WindowCloseRequest},
};
use std::sync::Arc;

pub(super) struct RegisteredHost {
    pub(super) facade: HostFacade<NodeHost>,
    pub(super) templates: Arc<TemplateCache>,
    pub(super) compute: ComputeLimits,
    pub(super) node: NodeIdentity,
}
impl LifecycleReceiver for RegisteredHost {
    async fn receive(&self, request: LifecycleRequest) -> Result<LifecycleResponse, ReceiveError> {
        self.facade.receive(request).await
    }
    async fn confirm(&self, result: LifecycleResponse) -> Result<LifecycleResponse, ReceiveError> {
        self.facade.confirm(result).await
    }
    async fn close(&self, request: WindowCloseRequest) -> Result<WindowCloseAck, ReceiveError> {
        self.facade.close(request).await
    }
}
impl DataReceiver for RegisteredHost {
    async fn receive_data(
        &self,
        identity: ControlIdentity,
        request: Request<Body>,
    ) -> Response<Body> {
        self.facade.receive_data(identity, request).await
    }
}
impl ValidationReceiver for RegisteredHost {
    async fn validation_evidence(
        &self,
        request: ValidationRequest,
    ) -> Result<ValidationEvidence, ReceiveError> {
        self.facade.validation_evidence(request).await
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
            observed_at_ms,
            free,
        })
    }
}
