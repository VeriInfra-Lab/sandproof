//! Explicit component-fixture capabilities, never production assembly.
use crate::status::{HostStatus, StatusReceiver, StatusRequest};
use crate::{CacheReceiver, DataReceiver, LifecycleReceiver, ReceiveError, ValidationReceiver};
use crate::{
    cache::{CacheAvailability, CacheRequest},
    template::{ValidationEvidence, ValidationRequest},
};
use axum::body::Body;
use hyper::{Request, Response};
use sandproof_protocol::lifecycle::{AuthenticatedCenterContext, LifecycleAccessIdentity};

/// Select this wrapper only for a component test with no artifact capability.
/// It delegates the original algorithms and explicitly rejects the new RPC;
/// the final host product must instead supply its mandatory real cache port.
pub struct RejectTemplateCache<H>(H);
impl<H> RejectTemplateCache<H> {
    #[must_use]
    pub const fn new(handler: H) -> Self {
        Self(handler)
    }
}
impl<H: LifecycleReceiver> LifecycleReceiver for RejectTemplateCache<H> {
    async fn observe_terminal(
        &self,
        intent: sandproof_protocol::lifecycle::AcceptedLifecycleIntent,
        context: sandproof_protocol::lifecycle::AuthenticatedCenterContext,
    ) -> Result<sandproof_protocol::lifecycle::TerminalLifecycleObservation, ReceiveError> {
        self.0.observe_terminal(intent, context).await
    }
    async fn observe_create(
        &self,
        intent: sandproof_protocol::lifecycle::AcceptedLifecycleIntent,
        context: sandproof_protocol::lifecycle::AuthenticatedCenterContext,
    ) -> Result<sandproof_protocol::lifecycle::CreateObservation, ReceiveError> {
        self.0.observe_create(intent, context).await
    }
    async fn record_lifecycle(
        &self,
        intent: sandproof_protocol::lifecycle::AcceptedLifecycleIntent,
        context: sandproof_protocol::lifecycle::AuthenticatedCenterContext,
    ) -> Result<sandproof_protocol::lifecycle::RecordedLifecycleIntent, ReceiveError> {
        self.0.record_lifecycle(intent, context).await
    }
}
impl<H: DataReceiver> DataReceiver for RejectTemplateCache<H> {
    async fn receive_data(
        &self,
        identity: LifecycleAccessIdentity,
        context: AuthenticatedCenterContext,
        request: Request<Body>,
    ) -> Response<Body> {
        self.0.receive_data(identity, context, request).await
    }
}
impl<H: ValidationReceiver> ValidationReceiver for RejectTemplateCache<H> {
    async fn validation_evidence(
        &self,
        request: ValidationRequest,
    ) -> Result<ValidationEvidence, ReceiveError> {
        self.0.validation_evidence(request).await
    }
}
impl<H: Send + Sync + 'static> CacheReceiver for RejectTemplateCache<H> {
    async fn cache_template(&self, _: CacheRequest) -> Result<CacheAvailability, ReceiveError> {
        Err(ReceiveError::Unavailable)
    }
}
// Component fixtures explicitly do not claim physical/time observation.
impl<H: Send + Sync + 'static> StatusReceiver for RejectTemplateCache<H> {
    async fn status(&self, _: StatusRequest) -> Result<HostStatus, ReceiveError> {
        Err(ReceiveError::Unavailable)
    }
}
