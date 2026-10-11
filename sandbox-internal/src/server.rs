use crate::{HTTP_HEADER_BUFFER_BYTES, HTTP_MAX_HEADERS, Limits, ServerTls, TransportError};
use http_body_util::{BodyExt, Full, Limited};
use hyper::{
    Method, Request, Response, StatusCode,
    body::{Bytes, Incoming},
    server::conn::http1,
    service::service_fn,
};
use hyper_util::rt::TokioIo;
use sandproof_protocol::internal::lifecycle::{
    LIFECYCLE_BODY_BYTES, LIFECYCLE_PATH, LifecycleRequest, LifecycleResponse,
};
use sandproof_protocol::internal::{
    NodeIdentity, PROBE_BODY_BYTES, PROBE_PATH, ProbeRequest, ProbeResponse,
};
use std::{convert::Infallible, sync::Arc};
use tokio::{net::TcpListener, sync::Semaphore, sync::oneshot, task::JoinSet, time::timeout};
use tokio_rustls::TlsAcceptor;

/// Effectful receiver after mutual authentication AND complete node binding.
/// Cancellation can leave durable effects: implementations must keep their
/// work budget until the actual worker exits, not just until this future drops.
pub trait LifecycleReceiver: Send + Sync + 'static {
    fn receive(
        &self,
        request: LifecycleRequest,
    ) -> impl std::future::Future<Output = Result<LifecycleResponse, ReceiveError>> + Send;
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

impl<H: LifecycleReceiver> InternalServer<H> {
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
                            let service = service_fn(move |request| route(request, node.clone(), Arc::clone(&handler)));
                            http1::Builder::new()
                                .keep_alive(false)
                                .max_headers(HTTP_MAX_HEADERS)
                                .max_buf_size(HTTP_HEADER_BUFFER_BYTES)
                                .serve_connection(TokioIo::new(stream), service)
                                .await
                                .map_err(|_| TransportError::Communication)
                        };
                        // Includes handshake, headers, body AND response drain.
                        timeout(limits.request_timeout, work)
                            .await
                            .unwrap_or(Err(TransportError::Deadline))
                    });
                }
            }
        };
        tasks.abort_all();
        while tasks.join_next().await.is_some() {}
        result
    }
}

async fn route<H: LifecycleReceiver>(
    request: Request<Incoming>,
    node: NodeIdentity,
    handler: Arc<H>,
) -> Result<Response<Full<Bytes>>, Infallible> {
    let lifecycle = request.uri().path() == LIFECYCLE_PATH;
    let limit = if lifecycle {
        LIFECYCLE_BODY_BYTES
    } else {
        PROBE_BODY_BYTES
    };
    let status =
        if (!lifecycle && request.uri().path() != PROBE_PATH) || request.uri().query().is_some() {
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
        return Ok(response(
            status,
            Bytes::from_static(b"internal request rejected"),
        ));
    }
    if request
        .headers()
        .get(hyper::header::CONTENT_LENGTH)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse::<u64>().ok())
        .is_some_and(|length| length > limit as u64)
    {
        return Ok(response(StatusCode::PAYLOAD_TOO_LARGE, Bytes::new()));
    }
    let Ok(body) = Limited::new(request.into_body(), limit).collect().await else {
        return Ok(response(StatusCode::BAD_REQUEST, Bytes::new()));
    };
    let bytes = body.to_bytes();
    if lifecycle {
        let Ok(request) = serde_json::from_slice::<LifecycleRequest>(&bytes) else {
            return Ok(response(StatusCode::BAD_REQUEST, Bytes::new()));
        };
        if request.validate(&node).is_err() {
            return Ok(response(StatusCode::CONFLICT, Bytes::new()));
        }
        let answer = match handler.receive(request.clone()).await {
            Ok(answer) if answer.validate_for(&request).is_ok() => answer,
            Ok(_) | Err(ReceiveError::Unavailable | ReceiveError::Capacity) => {
                return Ok(response(StatusCode::SERVICE_UNAVAILABLE, Bytes::new()));
            }
            Err(ReceiveError::Conflict) => return Ok(response(StatusCode::CONFLICT, Bytes::new())),
        };
        return Ok(json_response(&answer, limit));
    }
    let Ok(request) = serde_json::from_slice::<ProbeRequest>(&bytes) else {
        return Ok(response(StatusCode::BAD_REQUEST, Bytes::new()));
    };
    let Ok(answer) = ProbeResponse::for_request(&request, &node) else {
        return Ok(response(StatusCode::CONFLICT, Bytes::new()));
    };
    Ok(json_response(&answer, limit))
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
