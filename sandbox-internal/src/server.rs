use crate::{HTTP_HEADER_BUFFER_BYTES, HTTP_MAX_HEADERS, Limits, ServerTls, TransportError};
use http_body_util::{BodyExt, Full, Limited};
use hyper::{
    Method, Request, Response, StatusCode,
    body::{Bytes, Incoming},
    server::conn::http1,
    service::service_fn,
};
use hyper_util::rt::TokioIo;
use sandproof_protocol::internal::{
    NodeIdentity, PROBE_BODY_BYTES, PROBE_PATH, ProbeRequest, ProbeResponse,
};
use std::{convert::Infallible, sync::Arc};
use tokio::{net::TcpListener, sync::Semaphore, sync::oneshot, task::JoinSet, time::timeout};
use tokio_rustls::TlsAcceptor;

/// Only the registered center certificate can reach the read-only probe.
/// No generic host-command/URL/path endpoint or unauthenticated health exists.
#[derive(Clone)]
pub struct ProbeServer {
    tls: ServerTls,
    node: NodeIdentity,
    limits: Limits,
    connections: Arc<Semaphore>,
}

impl ProbeServer {
    #[must_use]
    pub fn new(tls: ServerTls, node: NodeIdentity, limits: Limits) -> Self {
        Self {
            tls,
            node,
            limits,
            connections: Arc::new(Semaphore::new(limits.max_connections)),
        }
    }

    /// Diagnostic occupancy, not an authorization or lifecycle observation.
    #[must_use]
    pub fn active_connections(&self) -> usize {
        self.limits.max_connections - self.connections.available_permits()
    }

    /// Accept with no waiting semaphore queue, including before TLS/header read.
    /// Completed tasks are reaped before admitting more connections. Shutdown
    /// cancels and joins only this server's read-only calls, not any host work.
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
                            let service = service_fn(move |request| probe(request, node.clone()));
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

async fn probe(
    request: Request<Incoming>,
    node: NodeIdentity,
) -> Result<Response<Full<Bytes>>, Infallible> {
    let status = if request.uri().path() != PROBE_PATH || request.uri().query().is_some() {
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
        .is_some_and(|length| length > PROBE_BODY_BYTES as u64)
    {
        return Ok(response(StatusCode::PAYLOAD_TOO_LARGE, Bytes::new()));
    }
    let Ok(body) = Limited::new(request.into_body(), PROBE_BODY_BYTES)
        .collect()
        .await
    else {
        return Ok(response(StatusCode::BAD_REQUEST, Bytes::new()));
    };
    let Ok(request) = serde_json::from_slice::<ProbeRequest>(&body.to_bytes()) else {
        return Ok(response(StatusCode::BAD_REQUEST, Bytes::new()));
    };
    let Ok(answer) = ProbeResponse::for_request(&request, &node) else {
        return Ok(response(StatusCode::CONFLICT, Bytes::new()));
    };
    let Ok(encoded) = serde_json::to_vec(&answer) else {
        return Ok(response(StatusCode::INTERNAL_SERVER_ERROR, Bytes::new()));
    };
    if encoded.len() > PROBE_BODY_BYTES {
        return Ok(response(StatusCode::INTERNAL_SERVER_ERROR, Bytes::new()));
    }
    let mut answer = response(StatusCode::OK, Bytes::from(encoded));
    answer.headers_mut().insert(
        hyper::header::CONTENT_TYPE,
        hyper::header::HeaderValue::from_static("application/json"),
    );
    Ok(answer)
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
