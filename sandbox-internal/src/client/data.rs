//! Stream original envd HTTP through one registered, pinned mTLS connection.
//! No lifecycle retry, response buffering, arbitrary URL or fresh VM is allowed.
use super::{
    Arc, Bytes, HTTP_HEADER_BUFFER_BYTES, HTTP_MAX_HEADERS, InternalClient, Request, TcpStream,
    TlsConnector, TokioIo, TransportError, http1, timeout,
};
use axum::body::Body;
use sandproof_data_transport::WriteIdle;
use sandproof_protocol::internal::{
    ControlIdentity, DATA_IDENTITY_BYTES, DATA_IDENTITY_HEADER, DATA_PATH_PREFIX,
};
use std::{
    future::Future,
    io,
    pin::Pin,
    task::{Context, Poll},
};
use tokio::sync::{OwnedSemaphorePermit, oneshot};
use tokio::time::{Instant, Sleep, sleep_until, timeout_at};

/// Body and connection independently retain capacity. Dropping the response
/// signals cancellation; the socket task retains capacity until the actual
/// connection future has been dropped, including canceled caller futures.
struct StreamLease {
    _permit: Arc<OwnedSemaphorePermit>,
    _route: Arc<dyn Send + Sync>,
    _stop: oneshot::Sender<()>,
}
struct LeasedBody {
    inner: Body,
    lease: Option<StreamLease>,
    deadline: Pin<Box<Sleep>>,
}
impl hyper::body::Body for LeasedBody {
    type Data = Bytes;
    type Error = axum::Error;

    fn poll_frame(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<hyper::body::Frame<Bytes>, Self::Error>>> {
        if self.lease.is_none() {
            return Poll::Ready(None);
        }
        if self.deadline.as_mut().poll(cx).is_ready() {
            self.lease.take();
            return Poll::Ready(Some(Err(axum::Error::new(io::Error::new(
                io::ErrorKind::TimedOut,
                "original sandbox maximum lifetime reached",
            )))));
        }
        let frame = Pin::new(&mut self.inner).poll_frame(cx);
        if matches!(frame, Poll::Ready(None | Some(Err(_)))) {
            self.lease.take();
        }
        frame
    }
}

impl InternalClient {
    /// Forward one already center-authorized SDK data request. The host still
    /// checks its ORIGINAL local current identity/permissions/lifetime. Only
    /// the protected endpoint and the original origin-form envd URI are used.
    /// The short RPC deadline covers connection/authentication, not a command
    /// stream's duration; original host idle and revocation rules remain active.
    /// `max_deadline` is derived from the fresh authority's original fixed
    /// maximum lifetime, not a renewed short deadline. It bounds a read-only
    /// black hole even when the host's cancellation cannot traverse the network.
    /// # Errors
    /// Invalid identity/URI, bounded overload, authentication and connection
    /// failures reject without fallback or a fresh lifecycle operation.
    pub async fn stream_data(
        &self,
        identity: &ControlIdentity,
        mut request: Request<Body>,
        route_lease: Arc<dyn Send + Sync>,
        max_deadline: Instant,
    ) -> Result<hyper::Response<Body>, TransportError> {
        if max_deadline <= Instant::now() {
            return Err(TransportError::Deadline);
        }
        identity
            .validate_node(&self.endpoint.node)
            .map_err(|_| TransportError::Rejected)?;
        if request.uri().scheme().is_some() || request.uri().authority().is_some() {
            return Err(TransportError::Rejected);
        }
        let identity = serde_json::to_vec(identity).map_err(|_| TransportError::Rejected)?;
        if identity.len() > DATA_IDENTITY_BYTES {
            return Err(TransportError::Rejected);
        }
        let path = request
            .uri()
            .path_and_query()
            .map_or("/", |path| path.as_str());
        if !path.starts_with('/') || path.len() > 4096 {
            return Err(TransportError::Rejected);
        }
        *request.uri_mut() = format!("{DATA_PATH_PREFIX}{path}")
            .parse()
            .map_err(|_| TransportError::Rejected)?;
        request.headers_mut().remove(DATA_IDENTITY_HEADER);
        request.headers_mut().insert(
            DATA_IDENTITY_HEADER,
            identity.try_into().map_err(|_| TransportError::Rejected)?,
        );
        request.headers_mut().remove("x-api-key");
        request.headers_mut().remove("x-access-token");
        request.headers_mut().insert(
            hyper::header::HOST,
            self.endpoint
                .server_name
                .to_str()
                .as_ref()
                .try_into()
                .map_err(|_| TransportError::Configuration)?,
        );
        request.headers_mut().insert(
            hyper::header::CONNECTION,
            hyper::header::HeaderValue::from_static("close"),
        );
        let permit = Arc::new(
            Arc::clone(&self.calls)
                .try_acquire_owned()
                .map_err(|_| TransportError::Overloaded)?,
        );
        let connection_deadline = max_deadline.min(Instant::now() + self.limits.request_timeout);
        let (mut sender, connection) = timeout_at(connection_deadline, self.data_connection())
            .await
            .map_err(|_| TransportError::Deadline)??;
        let (stop, shutdown) = oneshot::channel::<()>();
        let lease = StreamLease {
            _permit: Arc::clone(&permit),
            _route: Arc::clone(&route_lease),
            _stop: stop,
        };
        tokio::spawn(async move {
            let _permit = permit;
            let _route = route_lease;
            tokio::pin!(connection);
            tokio::select! {
                _ = shutdown => {},
                _ = &mut connection => {},
                () = sleep_until(max_deadline) => {},
            }
            // Connection is dropped before the original call permit.
        });
        let response = timeout_at(max_deadline, sender.send_request(request))
            .await
            .map_err(|_| TransportError::Deadline)?
            .map_err(|_| TransportError::Communication)?;
        Ok(response.map(|body| {
            Body::new(LeasedBody {
                inner: Body::new(body),
                lease: Some(lease),
                deadline: Box::pin(sleep_until(max_deadline)),
            })
        }))
    }

    async fn data_connection(
        &self,
    ) -> Result<
        (
            http1::SendRequest<Body>,
            impl std::future::Future<Output = Result<(), hyper::Error>> + Send + 'static,
        ),
        TransportError,
    > {
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
        http1::Builder::new()
            .max_headers(HTTP_MAX_HEADERS)
            .max_buf_size(HTTP_HEADER_BUFFER_BYTES)
            .handshake(TokioIo::new(WriteIdle::new(
                stream,
                Some(self.limits.request_timeout),
            )))
            .await
            .map_err(|_| TransportError::Communication)
    }
}
