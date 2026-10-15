//! One explicit guest HTTP assembly. Existing command/file admission, error
//! observations and responsibility remain in their original handlers. This does
//! not provision PID state, bind an address or install/enable a daemon.
use crate::{
    AccountSource, MetadataSource, Session, command_http, file_http, file_rpc_http,
    initialization_routes, listener,
};
use axum::{Router, body::Body, http::Request, response::Response};
use sandproof_guest_runtime::file_download::date::LocalZone;
use std::{fmt, future::Future, io, sync::Arc};
use tokio::net::TcpListener;
use tower::ServiceExt;

/// All settings, pools, account sources and VM/PID resources are constructed by
/// the trusted caller. Reusing this assembly preserves the command ID sequence.
pub struct Components<M, S, Z> {
    pub commands: command_http::CommandHttp<M, S>,
    pub files: file_http::FileHttp<M, S, Z>,
    pub filesystem: file_rpc_http::FileRpcHttp<M, S>,
}
pub struct GuestHttp<M, S, Z> {
    inner: Arc<Inner<M, S, Z>>,
}
struct Inner<M, S, Z> {
    initialization: Router,
    components: Components<M, S, Z>,
}
impl<M, S, Z> Clone for GuestHttp<M, S, Z> {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SessionMismatch;

/// Original typed failures remain owned until the listener finishes rejecting
/// the connection. No arbitrary contents or request paths enter Debug/logs.
pub enum Failure {
    Route,
    Command(Box<command_http::Failure>),
    Files(Box<file_http::Failure>),
    Filesystem(Box<file_rpc_http::Failure>),
}
impl fmt::Debug for Failure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("GuestHttpFailure").finish_non_exhaustive()
    }
}

impl<M: MetadataSource, S: AccountSource, Z: LocalZone + Send + Sync + 'static> GuestHttp<M, S, Z> {
    /// # Errors
    /// Rejects handlers using a different authorization state, even when their
    /// metadata and current token happen to match. No request/effect is started.
    pub fn new(
        session: Session<M>,
        components: Components<M, S, Z>,
    ) -> Result<Self, SessionMismatch> {
        if !components.commands.shares_session(&session)
            || !components.files.shares_session(&session)
            || !components.filesystem.shares_session(&session)
        {
            return Err(SessionMismatch);
        }
        Ok(Self {
            inner: Arc::new(Inner {
                initialization: initialization_routes(session),
                components,
            }),
        })
    }

    /// Dispatch once without reading, copying or normalizing the request body.
    /// The caller must bound connections/headers before invoking this method;
    /// each selected handler retains its own original admission and deadline.
    /// # Errors
    /// Unknown paths and unchanged component failures. There is deliberately no
    /// invented catch-all HTTP status; serve closes failed connections safely.
    pub async fn handle(&self, request: Request<Body>) -> Result<Response, Failure> {
        let components = &self.inner.components;
        match request.uri().path() {
            "/health" | "/init" => match self.inner.initialization.clone().oneshot(request).await {
                Ok(response) => Ok(response),
                Err(never) => match never {},
            },
            "/files" => components
                .files
                .handle(request)
                .await
                .map_err(|e| Failure::Files(Box::new(e))),
            path if path.starts_with("/process.Process/") => components
                .commands
                .handle(request)
                .await
                .map_err(|e| Failure::Command(Box::new(e))),
            path if path.starts_with("/filesystem.Filesystem/") => components
                .filesystem
                .handle(request)
                .await
                .map_err(|e| Failure::Filesystem(Box::new(e))),
            _ => Err(Failure::Route),
        }
    }

    /// Use the existing bounded HTTP/1 listener on a caller-bound private guest
    /// socket. Shutdown stops admission and drains transports under their old
    /// deadlines; it neither replays effects nor proves commands/files stopped.
    /// # Errors
    /// Listener acceptance failure; original handler failures stay connection
    /// local and are counted in the returned transport report.
    pub async fn serve(
        self,
        transport: listener::Listener,
        socket: TcpListener,
        shutdown: impl Future<Output = ()>,
    ) -> io::Result<listener::Report> {
        transport
            .serve(
                socket,
                move |request| {
                    let http = self.clone();
                    async move { http.handle(request).await }
                },
                shutdown,
            )
            .await
    }
}
