use crate::cache::{
    CACHE_PATH, CACHE_REQUEST_BYTES, CACHE_RESPONSE_BYTES, CacheAvailability, CacheReceiver,
    CacheRequest,
};
use crate::status::{
    HostStatus, STATUS_PATH, STATUS_REQUEST_BYTES, STATUS_RESPONSE_BYTES, StatusReceiver,
    StatusRequest,
};
use crate::template::{
    VALIDATION_PATH, VALIDATION_REQUEST_BODY_BYTES, VALIDATION_RESPONSE_BODY_BYTES,
    ValidationEvidence, ValidationReceiver, ValidationRequest,
};
use crate::{CertificatePin, Limits, ProtectedHttp, ProtectedServer, ServerTls, TransportError};
use axum::body::Body;
use http_body_util::{BodyExt, Full, Limited};
use hyper::{
    Method, Request, Response, StatusCode,
    body::{Bytes, Incoming},
};
use sandproof_protocol::internal::lifecycle::{
    LIFECYCLE_BODY_BYTES, LIFECYCLE_PATH, LifecycleRequest, LifecycleResponse,
    RESULT_CONFIRMATION_BODY_BYTES, RESULT_CONFIRMATION_PATH,
};
use sandproof_protocol::internal::window::{
    WINDOW_CLOSE_BODY_BYTES, WINDOW_CLOSE_PATH, WindowCloseAck, WindowCloseRequest,
};
use sandproof_protocol::internal::{
    ControlIdentity, DATA_IDENTITY_BYTES, DATA_IDENTITY_HEADER, DATA_PATH_PREFIX, NodeIdentity,
    PROBE_BODY_BYTES, PROBE_PATH, ProbeRequest, ProbeResponse,
};
use std::{convert::Infallible, sync::Arc};
use tokio::{net::TcpListener, sync::Semaphore, sync::oneshot, sync::watch, time::timeout};
mod lifecycle_v3;

/// Effectful receiver after mutual authentication AND complete node binding.
/// Cancellation can leave durable effects: implementations must keep their
/// work budget until the actual worker exits, not just until this future drops.
pub trait LifecycleReceiver: Send + Sync + 'static {
    /// Persist only through the original application. Unsupported receivers
    /// cannot acknowledge an echo; even a successful record is not Ready.
    fn record_v3(
        &self,
        _intent: sandproof_protocol::lifecycle::AcceptedLifecycleIntent,
        _context: sandproof_protocol::lifecycle::AuthenticatedCenterContext,
    ) -> impl std::future::Future<
        Output = Result<sandproof_protocol::lifecycle::RecordedLifecycleIntent, ReceiveError>,
    > + Send {
        std::future::ready(Err(ReceiveError::Unavailable))
    }
    fn receive(
        &self,
        request: LifecycleRequest,
    ) -> impl std::future::Future<Output = Result<LifecycleResponse, ReceiveError>> + Send;
    /// Accept only an exact saved FINAL result after its center commit; no
    /// fallback implementation can acknowledge unsupported durable compaction.
    fn confirm(
        &self,
        result: LifecycleResponse,
    ) -> impl std::future::Future<Output = Result<LifecycleResponse, ReceiveError>> + Send;
    /// Close only a fully confirmed terminal window in the original durable
    /// transaction. Missing support cannot fall back to an acknowledged echo.
    fn close(
        &self,
        request: WindowCloseRequest,
    ) -> impl std::future::Future<Output = Result<WindowCloseAck, ReceiveError>> + Send;
}

/// A data stream is authorized AGAIN by the original local application. Mutual
/// TLS and a matching node do not grant Exec/FileRead/FileWrite or name a VM.
/// The receiver must retain exact route/child leases until their real exits.
pub trait DataReceiver: Send + Sync + 'static {
    fn receive_data(
        &self,
        identity: ControlIdentity,
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
impl LifecycleReceiver for RejectLifecycle {
    async fn receive(&self, _request: LifecycleRequest) -> Result<LifecycleResponse, ReceiveError> {
        Err(ReceiveError::Unavailable)
    }
    async fn confirm(&self, _: LifecycleResponse) -> Result<LifecycleResponse, ReceiveError> {
        Err(ReceiveError::Unavailable)
    }
    async fn close(&self, _: WindowCloseRequest) -> Result<WindowCloseAck, ReceiveError> {
        Err(ReceiveError::Unavailable)
    }
}
impl DataReceiver for RejectLifecycle {
    async fn receive_data(&self, _: ControlIdentity, _: Request<Body>) -> Response<Body> {
        sandproof_data_transport::response(StatusCode::SERVICE_UNAVAILABLE)
    }
}
impl ValidationReceiver for RejectLifecycle {
    async fn validation_evidence(
        &self,
        _: ValidationRequest,
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
    lifecycle_v3: Option<crate::lifecycle_v3::CenterPeerRegistration>,
}

impl<H> Clone for InternalServer<H> {
    fn clone(&self) -> Self {
        Self {
            tls: self.tls.clone(),
            node: self.node.clone(),
            limits: self.limits,
            connections: Arc::clone(&self.connections),
            handler: Arc::clone(&self.handler),
            lifecycle_v3: self.lifecycle_v3.clone(),
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
            lifecycle_v3: None,
        }
    }

    /// Explicit protected assembly only; ordinary v2 construction stays closed.
    /// # Errors
    /// The original server node must match the protected peer registration.
    pub fn with_lifecycle_v3(
        mut self,
        registration: crate::lifecycle_v3::CenterPeerRegistration,
    ) -> Result<Self, TransportError> {
        if !registration.matches_node(&self.node) {
            return Err(TransportError::Configuration);
        }
        self.lifecycle_v3 = Some(registration);
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
                lifecycle_v3: self.lifecycle_v3.clone(),
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
    lifecycle_v3: Option<crate::lifecycle_v3::CenterPeerRegistration>,
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
        if request.uri().path() == sandproof_protocol::lifecycle::LIFECYCLE_PATH {
            let work =
                lifecycle_v3::record(request, peer, self.lifecycle_v3.as_ref(), &*self.handler);
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
            streaming,
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
    streaming: watch::Sender<bool>,
) -> Result<Response<Body>, Infallible> {
    if request.uri().path().starts_with(DATA_PATH_PREFIX) {
        return Ok(receive_data(request, node, &*handler, streaming).await);
    }
    let work = management_route(request, node, handler);
    Ok(timeout(limits.request_timeout, work)
        .await
        .unwrap_or_else(|_| response(StatusCode::REQUEST_TIMEOUT, Bytes::new()))
        .map(Body::new))
}

async fn receive_data<H: DataReceiver>(
    mut request: Request<Incoming>,
    node: NodeIdentity,
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
        let Ok(identity) = serde_json::from_slice::<ControlIdentity>(header.as_bytes()) else {
            return sandproof_data_transport::response(StatusCode::BAD_REQUEST);
        };
        identity
    };
    if identity.validate_node(&node).is_err() {
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
    handler.receive_data(identity, request.map(Body::new)).await
}

async fn management_route<
    H: LifecycleReceiver + ValidationReceiver + CacheReceiver + StatusReceiver,
>(
    request: Request<Incoming>,
    node: NodeIdentity,
    handler: Arc<H>,
) -> Response<Full<Bytes>> {
    let lifecycle = request.uri().path() == LIFECYCLE_PATH;
    let confirmation = request.uri().path() == RESULT_CONFIRMATION_PATH;
    let closing = request.uri().path() == WINDOW_CLOSE_PATH;
    let validation = request.uri().path() == VALIDATION_PATH;
    let cache = request.uri().path() == CACHE_PATH;
    let status_request = request.uri().path() == STATUS_PATH;
    let limit = if status_request {
        STATUS_REQUEST_BYTES
    } else if cache {
        CACHE_REQUEST_BYTES
    } else if validation {
        VALIDATION_REQUEST_BODY_BYTES
    } else if closing {
        WINDOW_CLOSE_BODY_BYTES
    } else if confirmation {
        RESULT_CONFIRMATION_BODY_BYTES
    } else if lifecycle {
        LIFECYCLE_BODY_BYTES
    } else {
        PROBE_BODY_BYTES
    };
    let status = if (!lifecycle
        && !confirmation
        && !closing
        && !validation
        && !cache
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
    if validation {
        return validation_evidence(&bytes, &node, &*handler).await;
    }
    if closing {
        return close_window(&bytes, &node, &*handler).await;
    }
    if confirmation {
        return confirm_result(&bytes, &node, &*handler).await;
    }
    if lifecycle {
        return receive_intent(&bytes, &node, &*handler).await;
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

async fn receive_intent<H: LifecycleReceiver>(
    bytes: &[u8],
    node: &NodeIdentity,
    handler: &H,
) -> Response<Full<Bytes>> {
    let Ok(request) = serde_json::from_slice::<LifecycleRequest>(bytes) else {
        return response(StatusCode::BAD_REQUEST, Bytes::new());
    };
    if request.validate(node).is_err() {
        return response(StatusCode::CONFLICT, Bytes::new());
    }
    let answer = handler.receive(request.clone()).await;
    match answer {
        Ok(answer) if answer.validate_for(&request).is_ok() => {
            json_response(&answer, LIFECYCLE_BODY_BYTES)
        }
        Err(ReceiveError::Conflict) => response(StatusCode::CONFLICT, Bytes::new()),
        Ok(_) | Err(ReceiveError::Unavailable | ReceiveError::Capacity) => {
            response(StatusCode::SERVICE_UNAVAILABLE, Bytes::new())
        }
    }
}

async fn confirm_result<H: LifecycleReceiver>(
    bytes: &[u8],
    node: &NodeIdentity,
    handler: &H,
) -> Response<Full<Bytes>> {
    let Ok(result) = serde_json::from_slice::<LifecycleResponse>(bytes) else {
        return response(StatusCode::BAD_REQUEST, Bytes::new());
    };
    if result.identity.validate_node(node).is_err() {
        return response(StatusCode::CONFLICT, Bytes::new());
    }
    match handler.confirm(result.clone()).await {
        Ok(answer) if answer == result => json_response(&answer, RESULT_CONFIRMATION_BODY_BYTES),
        Err(ReceiveError::Conflict) => response(StatusCode::CONFLICT, Bytes::new()),
        Ok(_) | Err(ReceiveError::Unavailable | ReceiveError::Capacity) => {
            response(StatusCode::SERVICE_UNAVAILABLE, Bytes::new())
        }
    }
}

async fn close_window<H: LifecycleReceiver>(
    bytes: &[u8],
    node: &NodeIdentity,
    handler: &H,
) -> Response<Full<Bytes>> {
    let Ok(request) = serde_json::from_slice::<WindowCloseRequest>(bytes) else {
        return response(StatusCode::BAD_REQUEST, Bytes::new());
    };
    if request.validate(node).is_err() {
        return response(StatusCode::CONFLICT, Bytes::new());
    }
    match handler.close(request.clone()).await {
        Ok(answer) if answer.validate_for(&request).is_ok() => {
            json_response(&answer, WINDOW_CLOSE_BODY_BYTES)
        }
        Err(ReceiveError::Conflict) => response(StatusCode::CONFLICT, Bytes::new()),
        Ok(_) | Err(ReceiveError::Unavailable | ReceiveError::Capacity) => {
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
    node: &NodeIdentity,
    handler: &H,
) -> Response<Full<Bytes>> {
    let Ok(request) = serde_json::from_slice::<ValidationRequest>(bytes) else {
        return response(StatusCode::BAD_REQUEST, Bytes::new());
    };
    if request.validate(node).is_err() {
        return response(StatusCode::CONFLICT, Bytes::new());
    }
    match handler.validation_evidence(request.clone()).await {
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
