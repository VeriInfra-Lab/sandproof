use crate::{HTTP_HEADER_BUFFER_BYTES, HTTP_MAX_HEADERS, Limits, ServerTls, TransportError};
use axum::body::Body;
use http_body_util::{BodyExt, Full, Limited};
use hyper::{
    Method, Request, Response, StatusCode,
    body::{Bytes, Incoming},
    server::conn::http1,
    service::service_fn,
};
use hyper_util::rt::{TokioIo, TokioTimer};
use sandproof_data_transport::WriteIdle;
use sandproof_protocol::internal::lifecycle::{
    LIFECYCLE_BODY_BYTES, LIFECYCLE_PATH, LifecycleRequest, LifecycleResponse,
    RESULT_CONFIRMATION_BODY_BYTES, RESULT_CONFIRMATION_PATH,
};
use sandproof_protocol::internal::{
    ControlIdentity, DATA_IDENTITY_BYTES, DATA_IDENTITY_HEADER, DATA_PATH_PREFIX, NodeIdentity,
    PROBE_BODY_BYTES, PROBE_PATH, ProbeRequest, ProbeResponse,
};
use std::{convert::Infallible, sync::Arc};
use tokio::{
    net::TcpListener, sync::Semaphore, sync::oneshot, sync::watch, task::JoinSet, time::timeout,
};
use tokio_rustls::TlsAcceptor;

/// Effectful receiver after mutual authentication AND complete node binding.
/// Cancellation can leave durable effects: implementations must keep their
/// work budget until the actual worker exits, not just until this future drops.
pub trait LifecycleReceiver: Send + Sync + 'static {
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
}
impl DataReceiver for RejectLifecycle {
    async fn receive_data(&self, _: ControlIdentity, _: Request<Body>) -> Response<Body> {
        sandproof_data_transport::response(StatusCode::SERVICE_UNAVAILABLE)
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
}

impl<H> Clone for InternalServer<H> {
    fn clone(&self) -> Self {
        Self {
            tls: self.tls.clone(),
            node: self.node.clone(),
            limits: self.limits,
            connections: Arc::clone(&self.connections),
            handler: Arc::clone(&self.handler),
        }
    }
}

impl<H: LifecycleReceiver + DataReceiver> InternalServer<H> {
    #[must_use]
    pub fn new(tls: ServerTls, node: NodeIdentity, limits: Limits, handler: H) -> Self {
        Self {
            tls,
            node,
            limits,
            connections: Arc::new(Semaphore::new(limits.max_connections)),
            handler: Arc::new(handler),
        }
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
        mut shutdown: oneshot::Receiver<()>,
    ) -> Result<(), TransportError> {
        let mut tasks = JoinSet::new();
        let result = loop {
            tokio::select! {
                biased;
                _ = &mut shutdown => break Ok(()),
                _ = tasks.join_next(), if !tasks.is_empty() => {},
                accepted = listener.accept() => {
                    let Ok((socket, _)) = accepted else {
                        break Err(TransportError::Communication);
                    };
                    let Ok(permit) = Arc::clone(&self.connections).try_acquire_owned() else {
                        // Cannot send HTTP until TLS is authenticated. Closing
                        // the excess socket creates neither task nor wait queue.
                        drop(socket);
                        continue;
                    };
                    let tls = self.tls.clone();
                    let node = self.node.clone();
                    let limits = self.limits;
                    let handler = Arc::clone(&self.handler);
                    tasks.spawn(async move {
                        let _permit = permit;
                        let (streaming, mut stream_status) = watch::channel(false);
                        let work = async {
                            let stream = timeout(
                                limits.handshake_timeout,
                                TlsAcceptor::from(Arc::clone(&tls.config)).accept(socket),
                            ).await
                                .map_err(|_| TransportError::Deadline)?
                                .map_err(|_| TransportError::Authentication)?;
                            tls.verify_peer(stream.get_ref().1.peer_certificates())?;
                            if stream.get_ref().1.alpn_protocol() != Some(b"http/1.1") {
                                return Err(TransportError::Authentication);
                            }
                            let service = service_fn(move |request| route(request, node.clone(), Arc::clone(&handler), limits, streaming.clone()));
                            http1::Builder::new()
                                .keep_alive(false)
                                .max_headers(HTTP_MAX_HEADERS)
                                .max_buf_size(HTTP_HEADER_BUFFER_BYTES)
                                .timer(TokioTimer::new())
                                .header_read_timeout(limits.request_timeout)
                                .serve_connection(TokioIo::new(WriteIdle::new(stream, Some(limits.request_timeout))), service)
                                .await
                                .map_err(|_| TransportError::Communication)
                        };
                        // Start the original finite request deadline BEFORE
                        // handshake/headers/body/dispatch, not only when the
                        // service is eventually polled. Only a parsed complete
                        // node-bound data request switches to original stream
                        // idle/lifetime/revocation rules. Unclassified traffic
                        // and partial management bodies still close the socket.
                        tokio::pin!(work);
                        tokio::select! {
                            result = &mut work => result,
                            Ok(()) = stream_status.changed() => work.await,
                            () = tokio::time::sleep(limits.request_timeout) => Err(TransportError::Deadline),
                        }
                    });
                }
            }
        };
        tasks.abort_all();
        while tasks.join_next().await.is_some() {}
        result
    }
}

async fn route<H: LifecycleReceiver + DataReceiver>(
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

async fn management_route<H: LifecycleReceiver>(
    request: Request<Incoming>,
    node: NodeIdentity,
    handler: Arc<H>,
) -> Response<Full<Bytes>> {
    let lifecycle = request.uri().path() == LIFECYCLE_PATH;
    let confirmation = request.uri().path() == RESULT_CONFIRMATION_PATH;
    let limit = if confirmation {
        RESULT_CONFIRMATION_BODY_BYTES
    } else if lifecycle {
        LIFECYCLE_BODY_BYTES
    } else {
        PROBE_BODY_BYTES
    };
    let status = if (!lifecycle && !confirmation && request.uri().path() != PROBE_PATH)
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

fn json_response(answer: &impl serde::Serialize, limit: usize) -> Response<Full<Bytes>> {
    let Ok(encoded) = serde_json::to_vec(answer) else {
        return response(StatusCode::INTERNAL_SERVER_ERROR, Bytes::new());
    };
    if encoded.len() > limit {
        return response(StatusCode::INTERNAL_SERVER_ERROR, Bytes::new());
    }
    let mut answer = response(StatusCode::OK, Bytes::from(encoded));
    answer.headers_mut().insert(
        hyper::header::CONTENT_TYPE,
        hyper::header::HeaderValue::from_static("application/json"),
    );
    answer
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
