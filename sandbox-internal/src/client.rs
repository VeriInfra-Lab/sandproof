use crate::template::{
    VALIDATION_PATH, VALIDATION_REQUEST_BODY_BYTES, VALIDATION_RESPONSE_BODY_BYTES,
    ValidationEvidence, ValidationRequest,
};
use crate::{ClientTls, HTTP_HEADER_BUFFER_BYTES, HTTP_MAX_HEADERS, Limits, TransportError};
use http_body_util::{BodyExt, Full, Limited};
use hyper::{Request, body::Bytes, client::conn::http1};
use hyper_util::rt::TokioIo;
use rustls::pki_types::ServerName;
use sandproof_protocol::internal::lifecycle::{
    LIFECYCLE_BODY_BYTES, LIFECYCLE_PATH, LifecycleRequest, LifecycleResponse,
    RESULT_CONFIRMATION_BODY_BYTES, RESULT_CONFIRMATION_PATH,
};
use sandproof_protocol::internal::window::{
    WINDOW_CLOSE_BODY_BYTES, WINDOW_CLOSE_PATH, WindowCloseAck, WindowCloseRequest,
};
use sandproof_protocol::internal::{
    NodeIdentity, PROBE_BODY_BYTES, PROBE_PATH, ProbeRequest, ProbeResponse,
};
use std::{net::SocketAddr, sync::Arc};
use tokio::{net::TcpStream, sync::Semaphore, time::timeout};
use tokio_rustls::TlsConnector;

mod cache;
mod data;
mod lifecycle_v3;
mod status;

/// Fixed protected registration; no URL, redirect, resolver or user target.
#[derive(Clone)]
pub struct RegisteredEndpoint {
    address: SocketAddr,
    server_name: ServerName<'static>,
    node: NodeIdentity,
}

impl RegisteredEndpoint {
    /// # Errors
    /// Rejects unusable addresses, IP-as-certificate-name and malformed names.
    pub fn new(
        address: SocketAddr,
        server_name: String,
        node: NodeIdentity,
    ) -> Result<Self, TransportError> {
        let server_name =
            ServerName::try_from(server_name).map_err(|_| TransportError::Configuration)?;
        if address.port() == 0
            || address.ip().is_unspecified()
            || address.ip().is_multicast()
            || !matches!(server_name, ServerName::DnsName(_))
        {
            return Err(TransportError::Configuration);
        }
        Ok(Self {
            address,
            server_name,
            node,
        })
    }
}

/// No connection pool, retry queue, response map or background driver to leak.
/// Clones share the same total outstanding-call budget through complete bodies.
#[derive(Clone)]
pub struct InternalClient {
    tls: ClientTls,
    endpoint: RegisteredEndpoint,
    limits: Limits,
    calls: Arc<Semaphore>,
}

impl InternalClient {
    #[must_use]
    pub fn new(tls: ClientTls, endpoint: RegisteredEndpoint, limits: Limits) -> Self {
        Self {
            tls,
            endpoint,
            limits,
            calls: Arc::new(Semaphore::new(limits.max_connections)),
        }
    }

    #[must_use]
    pub fn active_calls(&self) -> usize {
        self.limits.max_connections - self.calls.available_permits()
    }

    /// The exact protected endpoint identity, never a public target selector.
    #[must_use]
    pub const fn node_identity(&self) -> &NodeIdentity {
        &self.endpoint.node
    }

    /// One call only. No transport failure results in a automatic resubmission.
    /// # Errors
    /// Rejects foreign requests locally, overload, failed authentication,
    /// deadlines, rejection or invalid/uncorrelated/bounded responses.
    pub async fn probe(&self, request: &ProbeRequest) -> Result<ProbeResponse, TransportError> {
        request
            .validate(&self.endpoint.node)
            .map_err(|_| TransportError::Rejected)?;
        let _permit = self
            .calls
            .try_acquire()
            .map_err(|_| TransportError::Overloaded)?;
        timeout(self.limits.request_timeout, self.exchange(request))
            .await
            .map_err(|_| TransportError::Deadline)?
    }

    /// One authenticated high-level intent. No automatic retry. In particular,
    /// rejection/timeout is NOT a side-effect-free admission refusal.
    /// # Errors
    /// Bounded transport failures retain the original operation at the caller.
    pub async fn lifecycle(
        &self,
        request: &LifecycleRequest,
    ) -> Result<LifecycleResponse, TransportError> {
        request
            .validate(&self.endpoint.node)
            .map_err(|_| TransportError::Rejected)?;
        let _permit = self
            .calls
            .try_acquire()
            .map_err(|_| TransportError::Overloaded)?;
        let work = async {
            let encoded = serde_json::to_vec(request).map_err(|_| TransportError::Rejected)?;
            let body = self
                .exchange_bytes(
                    LIFECYCLE_PATH,
                    encoded,
                    LIFECYCLE_BODY_BYTES,
                    LIFECYCLE_BODY_BYTES,
                )
                .await?;
            let response: LifecycleResponse =
                serde_json::from_slice(&body).map_err(|_| TransportError::InvalidResponse)?;
            response
                .validate_for(request)
                .map_err(|_| TransportError::InvalidResponse)?;
            Ok(response)
        };
        timeout(self.limits.request_timeout, work)
            .await
            .map_err(|_| TransportError::Deadline)?
    }

    /// One exact durable-result confirmation. Lost acknowledgements only allow
    /// replay of this same result, never optimistic removal at the center.
    /// # Errors
    /// Authentication, capacity, deadline, rejection or mismatched echo fail.
    pub async fn confirm_result(
        &self,
        result: &LifecycleResponse,
    ) -> Result<LifecycleResponse, TransportError> {
        result
            .identity
            .validate_node(&self.endpoint.node)
            .map_err(|_| TransportError::Rejected)?;
        let _permit = self
            .calls
            .try_acquire()
            .map_err(|_| TransportError::Overloaded)?;
        let work = async {
            let encoded = serde_json::to_vec(result).map_err(|_| TransportError::Rejected)?;
            let body = self
                .exchange_bytes(
                    RESULT_CONFIRMATION_PATH,
                    encoded,
                    RESULT_CONFIRMATION_BODY_BYTES,
                    RESULT_CONFIRMATION_BODY_BYTES,
                )
                .await?;
            let answer: LifecycleResponse =
                serde_json::from_slice(&body).map_err(|_| TransportError::InvalidResponse)?;
            if answer != *result {
                return Err(TransportError::InvalidResponse);
            }
            Ok(answer)
        };
        timeout(self.limits.request_timeout, work)
            .await
            .map_err(|_| TransportError::Deadline)?
    }

    /// One exact window-close attempt; a timeout or rejection does not erase
    /// history or open a successor. Only the same durable close may be retried.
    /// # Errors
    /// Foreign node, authentication/capacity/deadline faults and altered replies
    /// return unknown/rejected transport results rather than an optimistic ack.
    pub async fn close_window(
        &self,
        request: &WindowCloseRequest,
    ) -> Result<WindowCloseAck, TransportError> {
        request
            .validate(&self.endpoint.node)
            .map_err(|_| TransportError::Rejected)?;
        let _permit = self
            .calls
            .try_acquire()
            .map_err(|_| TransportError::Overloaded)?;
        let work = async {
            let encoded = serde_json::to_vec(request).map_err(|_| TransportError::Rejected)?;
            let body = self
                .exchange_bytes(
                    WINDOW_CLOSE_PATH,
                    encoded,
                    WINDOW_CLOSE_BODY_BYTES,
                    WINDOW_CLOSE_BODY_BYTES,
                )
                .await?;
            let answer: WindowCloseAck =
                serde_json::from_slice(&body).map_err(|_| TransportError::InvalidResponse)?;
            answer
                .validate_for(request)
                .map_err(|_| TransportError::InvalidResponse)?;
            Ok(answer)
        };
        timeout(self.limits.request_timeout, work)
            .await
            .map_err(|_| TransportError::Deadline)?
    }

    async fn exchange(&self, request: &ProbeRequest) -> Result<ProbeResponse, TransportError> {
        let encoded = serde_json::to_vec(request).map_err(|_| TransportError::Rejected)?;
        let body = self
            .exchange_bytes(PROBE_PATH, encoded, PROBE_BODY_BYTES, PROBE_BODY_BYTES)
            .await?;
        let response: ProbeResponse =
            serde_json::from_slice(&body).map_err(|_| TransportError::InvalidResponse)?;
        response
            .validate_for(request)
            .map_err(|_| TransportError::InvalidResponse)?;
        Ok(response)
    }

    async fn exchange_bytes(
        &self,
        path: &'static str,
        encoded: Vec<u8>,
        request_limit: usize,
        response_limit: usize,
    ) -> Result<Bytes, TransportError> {
        if encoded.len() > request_limit {
            return Err(TransportError::Rejected);
        }
        let socket = TcpStream::connect(self.endpoint.address)
            .await
            .map_err(|_| TransportError::Communication)?;
        let stream = timeout(
            self.limits.handshake_timeout,
            TlsConnector::from(Arc::clone(&self.tls.config))
                .connect(self.endpoint.server_name.clone(), socket),
        )
        .await
        .map_err(|_| TransportError::Deadline)?
        .map_err(|_| TransportError::Authentication)?;
        self.tls
            .verify_peer(stream.get_ref().1.peer_certificates())?;
        if stream.get_ref().1.alpn_protocol() != Some(b"http/1.1") {
            return Err(TransportError::Authentication);
        }
        let (mut sender, connection) = http1::Builder::new()
            .max_headers(HTTP_MAX_HEADERS)
            .max_buf_size(HTTP_HEADER_BUFFER_BYTES)
            .handshake(TokioIo::new(stream))
            .await
            .map_err(|_| TransportError::Communication)?;
        let http_request = Request::post(path)
            .header(
                hyper::header::HOST,
                self.endpoint.server_name.to_str().as_ref(),
            )
            .header(hyper::header::CONTENT_TYPE, "application/json")
            .header(hyper::header::CONNECTION, "close")
            .body(Full::new(Bytes::from(encoded)))
            .map_err(|_| TransportError::Configuration)?;
        let exchange = async {
            let response = sender
                .send_request(http_request)
                .await
                .map_err(|_| TransportError::Communication)?;
            if response.status() != hyper::StatusCode::OK {
                return Err(TransportError::Rejected);
            }
            if response
                .headers()
                .get(hyper::header::CONTENT_TYPE)
                .map(hyper::header::HeaderValue::as_bytes)
                != Some(b"application/json")
                || response
                    .headers()
                    .contains_key(hyper::header::CONTENT_ENCODING)
            {
                return Err(TransportError::InvalidResponse);
            }
            let body = Limited::new(response.into_body(), response_limit)
                .collect()
                .await
                .map_err(|_| TransportError::InvalidResponse)?;
            Ok(body.to_bytes())
        };
        tokio::pin!(exchange);
        tokio::pin!(connection);
        tokio::select! {
            biased;
            result = &mut exchange => result,
            result = &mut connection => {
                result.map_err(|_| TransportError::Communication)?;
                exchange.await
            }
        }
        // Both futures and original TLS stream are dropped before permit return.
    }

    /// Obtain exact original engineering evidence, never a guest success report.
    /// # Errors
    /// Unknown/rejected/oversized responses leave publication unsettled.
    pub async fn validation_evidence(
        &self,
        request: &ValidationRequest,
    ) -> Result<ValidationEvidence, TransportError> {
        request
            .validate(&self.endpoint.node)
            .map_err(|_| TransportError::Rejected)?;
        let _permit = self
            .calls
            .try_acquire()
            .map_err(|_| TransportError::Overloaded)?;
        let work = async {
            let encoded = request.encoded().map_err(|_| TransportError::Rejected)?;
            let body = self
                .exchange_bytes(
                    VALIDATION_PATH,
                    encoded,
                    VALIDATION_REQUEST_BODY_BYTES,
                    VALIDATION_RESPONSE_BODY_BYTES,
                )
                .await?;
            let result: ValidationEvidence =
                serde_json::from_slice(&body).map_err(|_| TransportError::InvalidResponse)?;
            result
                .validate_for(&request.create, &request.terminal, &request.publication)
                .map_err(|_| TransportError::InvalidResponse)?;
            Ok(result)
        };
        timeout(self.limits.request_timeout, work)
            .await
            .map_err(|_| TransportError::Deadline)?
    }
}
