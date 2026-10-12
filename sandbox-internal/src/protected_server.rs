//! The original bounded TLS/HTTP accept loop, shared by host and artifact roles.
use crate::{
    CertificatePin, HTTP_HEADER_BUFFER_BYTES, HTTP_MAX_HEADERS, Limits, ServerTls, TransportError,
};
use axum::body::Body;
use hyper::{Request, Response, body::Incoming, server::conn::http1, service::service_fn};
use hyper_util::rt::{TokioIo, TokioTimer};
use sandproof_data_transport::WriteIdle;
use std::{convert::Infallible, sync::Arc};
use tokio::{
    net::TcpListener,
    sync::{Semaphore, oneshot, watch},
    task::JoinSet,
    time::timeout,
};
use tokio_rustls::TlsAcceptor;

/// Called only after chain/role-pin authentication. The exact peer pin is
/// supplied by TLS, not by a request header or descriptor.
pub trait ProtectedHttp: Send + Sync + 'static {
    fn respond(
        &self,
        peer: CertificatePin,
        request: Request<Incoming>,
        streaming: watch::Sender<bool>,
    ) -> impl std::future::Future<Output = Result<Response<Body>, Infallible>> + Send;
}

pub struct ProtectedServer<H> {
    tls: ServerTls,
    limits: Limits,
    connections: Arc<Semaphore>,
    handler: Arc<H>,
}
impl<H> Clone for ProtectedServer<H> {
    fn clone(&self) -> Self {
        Self {
            tls: self.tls.clone(),
            limits: self.limits,
            connections: Arc::clone(&self.connections),
            handler: Arc::clone(&self.handler),
        }
    }
}
impl<H: ProtectedHttp> ProtectedServer<H> {
    #[must_use]
    pub fn new(tls: ServerTls, limits: Limits, handler: H) -> Self {
        Self {
            tls,
            limits,
            connections: Arc::new(Semaphore::new(limits.max_connections)),
            handler: Arc::new(handler),
        }
    }
    pub(crate) fn from_parts(
        tls: ServerTls,
        limits: Limits,
        connections: Arc<Semaphore>,
        handler: H,
    ) -> Self {
        Self {
            tls,
            limits,
            connections,
            handler: Arc::new(handler),
        }
    }
    #[must_use]
    pub fn active_connections(&self) -> usize {
        self.limits.max_connections - self.connections.available_permits()
    }
    /// No waiting semaphore queue, including before TLS/header parsing. A
    /// qualified streaming handler must install its own total/idle deadline
    /// before switching the original short request deadline off. Shutdown
    /// drops and joins connection tasks, not accepted durable application work.
    /// # Errors
    /// Listener failure returns a communication error without replacing state.
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
                    let Ok((socket, _)) = accepted else { break Err(TransportError::Communication); };
                    let Ok(permit) = Arc::clone(&self.connections).try_acquire_owned() else { drop(socket); continue; };
                    let tls = self.tls.clone();
                    let limits = self.limits;
                    let handler = Arc::clone(&self.handler);
                    tasks.spawn(async move {
                        let _permit = permit;
                        let (streaming, mut stream_status) = watch::channel(false);
                        let work = async {
                            let stream = timeout(limits.handshake_timeout, TlsAcceptor::from(Arc::clone(&tls.config)).accept(socket)).await
                                .map_err(|_| TransportError::Deadline)?.map_err(|_| TransportError::Authentication)?;
                            let peer = tls.peer_pin(stream.get_ref().1.peer_certificates())?;
                            if stream.get_ref().1.alpn_protocol() != Some(b"http/1.1") { return Err(TransportError::Authentication); }
                            let service = service_fn(move |request| {
                                let handler = Arc::clone(&handler);
                                let streaming = streaming.clone();
                                async move { handler.respond(peer, request, streaming).await }
                            });
                            http1::Builder::new().keep_alive(false).max_headers(HTTP_MAX_HEADERS).max_buf_size(HTTP_HEADER_BUFFER_BYTES)
                                .timer(TokioTimer::new()).header_read_timeout(limits.request_timeout)
                                .serve_connection(TokioIo::new(WriteIdle::new(stream, Some(limits.request_timeout))), service).await
                                .map_err(|_| TransportError::Communication)
                        };
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
