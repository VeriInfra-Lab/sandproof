use crate::cache::{
    CACHE_PATH, CACHE_REQUEST_BYTES, CACHE_RESPONSE_BYTES, CacheAvailability, CacheReceiver,
    CacheRequest,
};
use crate::status::{
    HostStatus, STATUS_PATH, STATUS_REQUEST_BYTES, STATUS_RESPONSE_BYTES, StatusReceiver,
    StatusRequest,
};
use crate::template::{
    VALIDATION_PATH, VALIDATION_RESPONSE_BODY_BYTES,
    ValidationEvidence, ValidationReceiver, ValidationRequest,
};
use crate::{CertificatePin, Limits, ProtectedHttp, ProtectedServer, ServerTls, TransportError};
use axum::body::Body;
use http_body_util::{BodyExt, Full, Limited};
use hyper::{
    Method, Request, Response, StatusCode,
    body::{Bytes, Incoming},
};
use sandproof_protocol::internal::{
    DATA_IDENTITY_BYTES, DATA_IDENTITY_HEADER, DATA_PATH_PREFIX, NodeIdentity,
    PROBE_BODY_BYTES, PROBE_PATH, ProbeRequest, ProbeResponse,
};
use sandproof_protocol::lifecycle::{AuthenticatedCenterContext, LifecycleAccessIdentity};
use std::{convert::Infallible, sync::Arc};
use tokio::{net::TcpListener, sync::Semaphore, sync::oneshot, sync::watch, time::timeout};
mod lifecycle;

/// Effectful receiver after mutual authentication AND complete node binding.
/// Cancellation can leave durable effects: implementations must keep their
/// work budget until the actual worker exits, not just until this future drops.
pub trait LifecycleReceiver: Send + Sync + 'static {
    fn confirm_terminal(
        &self,
        _result: sandproof_protocol::lifecycle::TerminalLifecycleObservation,
        _context: sandproof_protocol::lifecycle::AuthenticatedCenterContext,
    ) -> impl std::future::Future<Output = Result<sandproof_protocol::lifecycle::ConfirmedLifecycleResult, ReceiveError>> + Send {
        std::future::ready(Err(ReceiveError::Unavailable))
    }
    fn close_window(
        &self,
        _request: sandproof_protocol::lifecycle::LifecycleWindowClose,
        _context: sandproof_protocol::lifecycle::AuthenticatedCenterContext,
    ) -> impl std::future::Future<Output = Result<sandproof_protocol::lifecycle::ClosedLifecycleWindow, ReceiveError>> + Send {
        std::future::ready(Err(ReceiveError::Unavailable))
    }
    /// Read the same current Destroy; unsupported fixtures cannot acknowledge
    /// cleanup. The original provider retains all physical/material conditions.
    fn observe_terminal(
        &self,
        _intent: sandproof_protocol::lifecycle::AcceptedLifecycleIntent,
        _context: sandproof_protocol::lifecycle::AuthenticatedCenterContext,
    ) -> impl std::future::Future<Output = Result<sandproof_protocol::lifecycle::TerminalLifecycleObservation, ReceiveError>> + Send {
        std::future::ready(Err(ReceiveError::Unavailable))
    }
    /// One original readonly Create observation; unsupported receivers refuse.
    fn observe_create(
        &self,
        _intent: sandproof_protocol::lifecycle::AcceptedLifecycleIntent,
        _context: sandproof_protocol::lifecycle::AuthenticatedCenterContext,
    ) -> impl std::future::Future<
        Output = Result<sandproof_protocol::lifecycle::CreateObservation, ReceiveError>,
    > + Send {
        std::future::ready(Err(ReceiveError::Unavailable))
    }
    /// Persist only through the original application. Unsupported receivers
    /// cannot acknowledge an echo; even a successful record is not Ready.
    fn record_lifecycle(
        &self,
        _intent: sandproof_protocol::lifecycle::AcceptedLifecycleIntent,
        _context: sandproof_protocol::lifecycle::AuthenticatedCenterContext,
    ) -> impl std::future::Future<
        Output = Result<sandproof_protocol::lifecycle::RecordedLifecycleIntent, ReceiveError>,
    > + Send {
        std::future::ready(Err(ReceiveError::Unavailable))
    }
}

/// A data stream is authorized AGAIN by the original local application. Mutual
/// TLS and a matching node do not grant Exec/FileRead/FileWrite or name a VM.
/// The receiver must retain exact route/child leases until their real exits.
pub trait DataReceiver: Send + Sync + 'static {
    fn receive_data(
        &self,
        identity: LifecycleAccessIdentity,
        context: AuthenticatedCenterContext,
        request: Request<Body>,
    ) -> impl std::future::Future<Output = Response<Body>> + Send;
}

/// HTTP errors carry no proof that local effects were absent.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReceiveError {
    Conflict,
    Capacity,
    Unavailable,
}

/// Diagnostic receivers reject lifecycle calls; no public deployment fallback.
pub struct RejectLifecycle;
impl LifecycleReceiver for RejectLifecycle {}
impl DataReceiver for RejectLifecycle {
    async fn receive_data(
        &self,
        _: LifecycleAccessIdentity,
        _: AuthenticatedCenterContext,
        _: Request<Body>,
    ) -> Response<Body> {
        sandproof_data_transport::response(StatusCode::SERVICE_UNAVAILABLE)
    }
}
impl ValidationReceiver for RejectLifecycle {
    async fn validation_evidence(
        &self,
        _: ValidationRequest,
        _: AuthenticatedCenterContext,
    ) -> Result<ValidationEvidence, ReceiveError> {
        Err(ReceiveError::Unavailable)
    }
}
impl CacheReceiver for RejectLifecycle {
    async fn cache_template(&self, _: CacheRequest) -> Result<CacheAvailability, ReceiveError> {
        Err(ReceiveError::Unavailable)
    }
}
impl StatusReceiver for RejectLifecycle {
    async fn status(&self, _: StatusRequest) -> Result<HostStatus, ReceiveError> {
        Err(ReceiveError::Unavailable)
    }
}

/// Only the registered center certificate can reach either internal endpoint.
/// No generic host-command/URL/path endpoint or unauthenticated health exists.
pub struct InternalServer<H> {
    tls: ServerTls,
    node: NodeIdentity,
    limits: Limits,
    connections: Arc<Semaphore>,
    handler: Arc<H>,
    lifecycle: Option<crate::lifecycle::CenterPeerRegistration>,
}

impl<H> Clone for InternalServer<H> {
    fn clone(&self) -> Self {
        Self {
            tls: self.tls.clone(),
            node: self.node.clone(),
            limits: self.limits,
            connections: Arc::clone(&self.connections),
            handler: Arc::clone(&self.handler),
            lifecycle: self.lifecycle.clone(),
        }
    }
}

impl<H: LifecycleReceiver + DataReceiver + ValidationReceiver + CacheReceiver + StatusReceiver>
    InternalServer<H>
{
    #[must_use]
    pub fn new(tls: ServerTls, node: NodeIdentity, limits: Limits, handler: H) -> Self {
        Self {
            tls,
            node,
            limits,
            connections: Arc::new(Semaphore::new(limits.max_connections)),
            handler: Arc::new(handler),
            lifecycle: None,
        }
    }

    /// Bind the only lifecycle route to its protected current center.
    /// # Errors
    /// The original server node must match the protected peer registration.
    pub fn with_center_peer(
        mut self,
        registration: crate::lifecycle::CenterPeerRegistration,
    ) -> Result<Self, TransportError> {
        if !registration.matches_node(&self.node) {
            return Err(TransportError::Configuration);
        }
        self.lifecycle = Some(registration);
        Ok(self)
    }

    /// Diagnostic occupancy, not an authorization or lifecycle observation.
    #[must_use]
    pub fn active_connections(&self) -> usize {
        self.limits.max_connections - self.connections.available_permits()
    }

    /// Accept with no waiting semaphore queue, including before TLS/header read.
    /// Completed tasks are reaped before admitting more connections. Shutdown
    /// cancels and joins only connection tasks, not accepted durable host work.
    /// # Errors
    /// Returns a fixed communication class for listener failures.
    pub async fn serve(
        &self,
        listener: TcpListener,
        shutdown: oneshot::Receiver<()>,
    ) -> Result<(), TransportError> {
        ProtectedServer::from_parts(
            self.tls.clone(),
            self.limits,
            Arc::clone(&self.connections),
            LifecycleHttp {
                node: self.node.clone(),
                handler: Arc::clone(&self.handler),
                limits: self.limits,
                lifecycle: self.lifecycle.clone(),
            },
        )
        .serve(listener, shutdown)
        .await
    }
}

struct LifecycleHttp<H> {
    node: NodeIdentity,
    handler: Arc<H>,
    limits: Limits,
    lifecycle: Option<crate::lifecycle::CenterPeerRegistration>,
}
impl<H: LifecycleReceiver + DataReceiver + ValidationReceiver + CacheReceiver + StatusReceiver>
    ProtectedHttp for LifecycleHttp<H>
{
    async fn respond(
        &self,
        peer: CertificatePin,
        request: Request<Incoming>,
        streaming: watch::Sender<bool>,
    ) -> Result<Response<Body>, Infallible> {
        if request.uri().path().starts_with(DATA_PATH_PREFIX) {
            let Some(registration) = self.lifecycle.as_ref() else {
                return Ok(sandproof_data_transport::response(StatusCode::SERVICE_UNAVAILABLE));
            };
            let Ok(context) = registration.context_for_verified_peer(peer) else {
                return Ok(sandproof_data_transport::response(StatusCode::UNAUTHORIZED));
            };
            return Ok(receive_data(request, context, &*self.handler, streaming).await);
        }
        if request.uri().path() == sandproof_protocol::lifecycle::LIFECYCLE_PATH
            || request.uri().path() == sandproof_protocol::lifecycle::CREATE_OBSERVATION_PATH
            || request.uri().path() == sandproof_protocol::lifecycle::TERMINAL_OBSERVATION_PATH
            || request.uri().path() == sandproof_protocol::lifecycle::RESULT_CONFIRMATION_PATH
            || request.uri().path() == sandproof_protocol::lifecycle::WINDOW_CLOSE_PATH
            || request.uri().path() == VALIDATION_PATH
        {
            let call = if request.uri().path() == sandproof_protocol::lifecycle::CREATE_OBSERVATION_PATH {
                lifecycle::Call::CreateObservation
            } else if request.uri().path() == sandproof_protocol::lifecycle::TERMINAL_OBSERVATION_PATH {
                lifecycle::Call::TerminalObservation
            } else if request.uri().path() == sandproof_protocol::lifecycle::RESULT_CONFIRMATION_PATH {
                lifecycle::Call::Confirm
            } else if request.uri().path() == sandproof_protocol::lifecycle::WINDOW_CLOSE_PATH {
                lifecycle::Call::Close
            } else if request.uri().path() == VALIDATION_PATH {
                lifecycle::Call::Validation
            } else { lifecycle::Call::Record };
            let work = lifecycle::respond(request, peer, self.lifecycle.as_ref(), &*self.handler, call);
            return Ok(timeout(self.limits.request_timeout, work)
                .await
                .unwrap_or_else(|_| response(StatusCode::REQUEST_TIMEOUT, Bytes::new()))
                .map(Body::new));
        }
        route(
            request,
            self.node.clone(),
            Arc::clone(&self.handler),
            self.limits,
        )
        .await
    }
}

async fn route<
    H: LifecycleReceiver + DataReceiver + ValidationReceiver + CacheReceiver + StatusReceiver,
>(
    request: Request<Incoming>,
    node: NodeIdentity,
    handler: Arc<H>,
    limits: Limits,
) -> Result<Response<Body>, Infallible> {
    let work = management_route(request, node, handler);
    Ok(timeout(limits.request_timeout, work)
        .await
        .unwrap_or_else(|_| response(StatusCode::REQUEST_TIMEOUT, Bytes::new()))
        .map(Body::new))
}

async fn receive_data<H: DataReceiver>(
    mut request: Request<Incoming>,
    context: AuthenticatedCenterContext,
    handler: &H,
    streaming: watch::Sender<bool>,
) -> Response<Body> {
    let identity = {
        let mut headers = request.headers().get_all(DATA_IDENTITY_HEADER).iter();
        let Some(header) = headers.next() else {
            return sandproof_data_transport::response(StatusCode::UNAUTHORIZED);
        };
        if headers.next().is_some() || header.as_bytes().len() > DATA_IDENTITY_BYTES {
            return sandproof_data_transport::response(StatusCode::BAD_REQUEST);
        }
        let Ok(identity) = serde_json::from_slice::<LifecycleAccessIdentity>(header.as_bytes()) else {
            return sandproof_data_transport::response(StatusCode::BAD_REQUEST);
        };
        identity
    };
    if identity.validate(&context).is_err() {
        return sandproof_data_transport::response(StatusCode::CONFLICT);
    }
    let path = request
        .uri()
        .path_and_query()
        .map_or("", |path| path.as_str());
    let Some(path) = path
        .strip_prefix(DATA_PATH_PREFIX)
        .filter(|path| path.starts_with('/'))
    else {
        return sandproof_data_transport::response(StatusCode::NOT_FOUND);
    };
    let Ok(uri) = path.parse() else {
        return sandproof_data_transport::response(StatusCode::BAD_REQUEST);
    };
    *request.uri_mut() = uri;
    request.headers_mut().remove(DATA_IDENTITY_HEADER);
    request.headers_mut().remove("x-api-key");
    request.headers_mut().remove("x-access-token");
    let _ = streaming.send(true);
    handler.receive_data(identity, context, request.map(Body::new)).await
}

async fn management_route<
    H: LifecycleReceiver + ValidationReceiver + CacheReceiver + StatusReceiver,
>(
    request: Request<Incoming>,
    node: NodeIdentity,
    handler: Arc<H>,
) -> Response<Full<Bytes>> {
    let cache = request.uri().path() == CACHE_PATH;
    let status_request = request.uri().path() == STATUS_PATH;
    let limit = if status_request {
        STATUS_REQUEST_BYTES
    } else if cache {
        CACHE_REQUEST_BYTES
    } else {
        PROBE_BODY_BYTES
    };
    let status = if (!cache
        && !status_request
        && request.uri().path() != PROBE_PATH)
        || request.uri().query().is_some()
    {
        Some(StatusCode::NOT_FOUND)
    } else if request.method() != Method::POST {
        Some(StatusCode::METHOD_NOT_ALLOWED)
    } else if request
        .headers()
        .get(hyper::header::CONTENT_TYPE)
        .map(hyper::header::HeaderValue::as_bytes)
        != Some(b"application/json")
        || request
            .headers()
            .contains_key(hyper::header::CONTENT_ENCODING)
    {
        Some(StatusCode::UNSUPPORTED_MEDIA_TYPE)
    } else {
        None
    };
    if let Some(status) = status {
        return response(status, Bytes::from_static(b"internal request rejected"));
    }
    if request
        .headers()
        .get(hyper::header::CONTENT_LENGTH)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse::<u64>().ok())
        .is_some_and(|length| length > limit as u64)
    {
        return response(StatusCode::PAYLOAD_TOO_LARGE, Bytes::new());
    }
    let Ok(body) = Limited::new(request.into_body(), limit).collect().await else {
        return response(StatusCode::BAD_REQUEST, Bytes::new());
    };
    let bytes = body.to_bytes();
    if status_request {
        return node_status(&bytes, &node, &*handler).await;
    }
    if cache {
        return cache_template(&bytes, &node, &*handler).await;
    }
    let Ok(request) = serde_json::from_slice::<ProbeRequest>(&bytes) else {
        return response(StatusCode::BAD_REQUEST, Bytes::new());
    };
    let Ok(answer) = ProbeResponse::for_request(&request, &node) else {
        return response(StatusCode::CONFLICT, Bytes::new());
    };
    json_response(&answer, limit)
}
async fn node_status<H: StatusReceiver>(
    bytes: &[u8],
    node: &NodeIdentity,
    handler: &H,
) -> Response<Full<Bytes>> {
    let Ok(request) = serde_json::from_slice::<StatusRequest>(bytes) else {
        return response(StatusCode::BAD_REQUEST, Bytes::new());
    };
    if request.validate(node).is_err() {
        return response(StatusCode::CONFLICT, Bytes::new());
    }
    match handler.status(request.clone()).await {
        Ok(answer) if answer.validate_for(&request).is_ok() => {
            json_response(&answer, STATUS_RESPONSE_BYTES)
        }
        Err(ReceiveError::Conflict) => response(StatusCode::CONFLICT, Bytes::new()),
        Ok(_) | Err(ReceiveError::Capacity | ReceiveError::Unavailable) => {
            response(StatusCode::SERVICE_UNAVAILABLE, Bytes::new())
        }
    }
}
async fn cache_template<H: CacheReceiver>(
    bytes: &[u8],
    node: &NodeIdentity,
    handler: &H,
) -> Response<Full<Bytes>> {
    let Ok(request) = serde_json::from_slice::<CacheRequest>(bytes) else {
        return response(StatusCode::BAD_REQUEST, Bytes::new());
    };
    if request.validate(node).is_err() {
        return response(StatusCode::CONFLICT, Bytes::new());
    }
    match handler.cache_template(request.clone()).await {
        Ok(answer) if answer.validate_for(&request).is_ok() => {
            json_response(&answer, CACHE_RESPONSE_BYTES)
        }
        Err(ReceiveError::Conflict) => response(StatusCode::CONFLICT, Bytes::new()),
        Ok(_) | Err(ReceiveError::Capacity | ReceiveError::Unavailable) => {
            response(StatusCode::SERVICE_UNAVAILABLE, Bytes::new())
        }
    }
}

fn json_response(answer: &impl serde::Serialize, limit: usize) -> Response<Full<Bytes>> {
    let Ok(encoded) =
        sandproof_controller_service::template_registry::evidence::encode_bounded(answer, limit)
    else {
        return response(StatusCode::INTERNAL_SERVER_ERROR, Bytes::new());
    };
    let mut answer = response(StatusCode::OK, Bytes::from(encoded));
    answer.headers_mut().insert(
        hyper::header::CONTENT_TYPE,
        hyper::header::HeaderValue::from_static("application/json"),
    );
    answer
}

async fn validation_evidence<H: ValidationReceiver>(
    bytes: &[u8],
    context: AuthenticatedCenterContext,
    handler: &H,
) -> Response<Full<Bytes>> {
    let Ok(request) = serde_json::from_slice::<ValidationRequest>(bytes) else {
        return response(StatusCode::BAD_REQUEST, Bytes::new());
    };
    if request.validate_context(&context).is_err() {
        return response(StatusCode::CONFLICT, Bytes::new());
    }
    match handler.validation_evidence(request.clone(), context).await {
        Ok(answer)
            if answer
                .validate_for(&request.create, &request.terminal, &request.publication)
                .is_ok() =>
        {
            json_response(&answer, VALIDATION_RESPONSE_BODY_BYTES)
        }
        Err(ReceiveError::Conflict) => response(StatusCode::CONFLICT, Bytes::new()),
        Ok(_) | Err(ReceiveError::Unavailable | ReceiveError::Capacity) => {
            response(StatusCode::SERVICE_UNAVAILABLE, Bytes::new())
        }
    }
}

fn response(status: StatusCode, body: Bytes) -> Response<Full<Bytes>> {
    let mut response = Response::new(Full::new(body));
    *response.status_mut() = status;
    response.headers_mut().insert(
        hyper::header::CONNECTION,
        hyper::header::HeaderValue::from_static("close"),
    );
    response
}
