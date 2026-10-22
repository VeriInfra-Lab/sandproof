//! The one production host receiver. Lifecycle and data use the SAME original
//! application; cloned lifecycle handles retain one non-waiting work gate.
use crate::{
    HostLifecycle,
    data_gateway::{self, Gateway, TransferLimits},
};
use axum::{
    body::Body,
    http::{Request, Response},
};
use sandproof_api::admission::RequestAdmission;
use sandproof_controller_service::application::{ControllerApplication, ControllerHost};
use sandproof_internal::template::{ValidationEvidence, ValidationReceiver, ValidationRequest};
use sandproof_internal::{DataReceiver, LifecycleReceiver, ReceiveError};
use sandproof_protocol::internal::{
    ControlIdentity,
    lifecycle::{LifecycleRequest, LifecycleResponse},
    window::{WindowCloseAck, WindowCloseRequest},
};
use std::sync::Arc;

pub struct HostFacade<H: ControllerHost> {
    application: Arc<ControllerApplication<H>>,
    lifecycle: HostLifecycle<H>,
    gateway: Arc<Gateway>,
}
impl<H: ControllerHost> Clone for HostFacade<H> {
    fn clone(&self) -> Self {
        Self {
            application: Arc::clone(&self.application),
            lifecycle: self.lifecycle.clone(),
            gateway: Arc::clone(&self.gateway),
        }
    }
}
impl<H: ControllerHost + Send + 'static> HostFacade<H> {
    /// Protected assembly supplies the original fully checked application and
    /// derived finite internal request budget. No caller keys or host URLs.
    /// # Errors
    /// Refuses invalid explicit data-transfer limits before serving.
    pub fn new(
        application: Arc<ControllerApplication<H>>,
        transfers: TransferLimits,
        requests: Arc<RequestAdmission>,
    ) -> Result<Self, &'static str> {
        let gateway = Arc::new(Gateway::with_admission(transfers, requests)?);
        Ok(Self {
            lifecycle: HostLifecycle::new(Arc::clone(&application)),
            application,
            gateway,
        })
    }

    /// Every background driver/reception clone shares the original work gate.
    /// This does not construct another worker or application.
    #[must_use]
    pub fn lifecycle(&self) -> HostLifecycle<H> {
        self.lifecycle.clone()
    }
}
impl<H: ControllerHost + Send + 'static> LifecycleReceiver for HostFacade<H> {
    async fn record_v3(
        &self,
        intent: sandproof_protocol::lifecycle::AcceptedLifecycleIntent,
        context: sandproof_protocol::lifecycle::AuthenticatedCenterContext,
    ) -> Result<sandproof_protocol::lifecycle::RecordedLifecycleIntent, ReceiveError> {
        self.lifecycle.record_v3(intent, context).await
    }
    async fn receive(&self, request: LifecycleRequest) -> Result<LifecycleResponse, ReceiveError> {
        self.lifecycle.receive(request).await
    }
    async fn confirm(
        &self,
        response: LifecycleResponse,
    ) -> Result<LifecycleResponse, ReceiveError> {
        self.lifecycle.confirm(response).await
    }
    async fn close(&self, request: WindowCloseRequest) -> Result<WindowCloseAck, ReceiveError> {
        self.lifecycle.close(request).await
    }
}
impl<H: ControllerHost + Send + 'static> DataReceiver for HostFacade<H> {
    async fn receive_data(
        &self,
        identity: ControlIdentity,
        request: Request<Body>,
    ) -> Response<Body> {
        data_gateway::handle_internal(
            Arc::clone(&self.application),
            Arc::clone(&self.gateway),
            identity,
            request,
        )
        .await
    }
}
impl<H: ControllerHost + Send + 'static> ValidationReceiver for HostFacade<H> {
    async fn validation_evidence(
        &self,
        request: ValidationRequest,
    ) -> Result<ValidationEvidence, ReceiveError> {
        self.lifecycle.validation_evidence(request).await
    }
}
