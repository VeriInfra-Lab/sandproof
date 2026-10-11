//! Fail-closed native envd gateway. The user sees neither the VM network namespace
//! nor a host port: each request first checks the durable core authority.

use std::{
    error::Error,
    io,
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
    time::Duration,
};

use axum::{
    body::Body,
    http::{Request, Response, StatusCode},
};
use hyper::client::conn::http1;
use hyper_util::rt::TokioIo;
use sandproof_api::admission::{RequestAdmission, RequestClass, RequestLease};
use sandproof_controller_service::application::{
    ApplicationError as PortError, ControllerApplication, ControllerHost, DataAccessPort,
    DataAuthorization, DataKind, DataPermit,
};
use sandproof_protocol::internal::ControlIdentity;
use tokio::{
    io::{AsyncRead, AsyncWrite, ReadBuf},
    process::{ChildStdin, ChildStdout},
};

mod admission;
mod drain;
pub use admission::Gateway;
#[cfg(all(target_os = "linux", test, feature = "sdk-contract-tests"))]
use sandproof_data_transport::{FILE_REQUEST_LIMIT, RESPONSE_LIMIT};
pub use sandproof_data_transport::{GatewayListener, TransferLimits};
use sandproof_data_transport::{
    WriteIdle, access_token, control_request, forward_request, hold_response, prepare_body,
    request_kind, response, tunnel, until_revoked,
};

const ENVD_ADDRESS: &str = "172.16.0.2:49983";

struct StdioTunnel {
    read: ChildStdout,
    write: ChildStdin,
}

impl AsyncRead for StdioTunnel {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        Pin::new(&mut self.read).poll_read(cx, buf)
    }
}

impl AsyncWrite for StdioTunnel {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        Pin::new(&mut self.write).poll_write(cx, buf)
    }
    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.write).poll_flush(cx)
    }
    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.write).poll_shutdown(cx)
    }
}

/// SDK data requests carry only the sandbox's envd bearer token. It is
/// resolved to one running sandbox, and the core checks effective Exec or
/// FileRead/FileWrite permission before launching a namespace-local relay.
pub async fn handle<P: DataAccessPort>(
    port: Arc<P>,
    gateway: Option<Arc<Gateway>>,
    request: Request<Body>,
) -> Response<Body> {
    let (kind, file_request) = match request_kind(&request) {
        Ok(kind) => kind,
        Err(status) => return response(status),
    };
    let token = match access_token(request.headers()) {
        Ok(token) => token,
        Err(status) => return response(status),
    };
    let authorization_port = Arc::clone(&port);
    handle_prepared(port, gateway, request, (kind, file_request), move |kind| {
        authorization_port
            .data_permit(&token, kind)
            .map(|permit| (permit, None))
    })
    .await
}

/// Complete authenticated binding, never a public SDK token or guessed VM ID.
struct NativeDataPort<H: ControllerHost> {
    application: Arc<ControllerApplication<H>>,
    identity: ControlIdentity,
}
impl<H: ControllerHost> NativeDataPort<H> {
    fn data_permit(&self, kind: DataKind) -> Result<(DataPermit, Option<String>), PortError> {
        self.application
            .internal_data_authorization(&self.identity, kind)
            .map(|(permit, secret)| (permit, Some(secret)))
    }
}

/// `InternalServer` invokes this only AFTER mTLS and complete node/header checks.
/// The native application checks full durable current identity again. Only the
/// original guest credential is forwarded; it never returns to the center.
pub async fn handle_internal<H: ControllerHost + Send + 'static>(
    application: Arc<ControllerApplication<H>>,
    gateway: Arc<Gateway>,
    identity: ControlIdentity,
    request: Request<Body>,
) -> Response<Body> {
    let selection = match request_kind(&request) {
        Ok(kind) => kind,
        Err(status) => return response(status),
    };
    let port = NativeDataPort {
        application: Arc::clone(&application),
        identity,
    };
    handle_prepared(
        application,
        Some(gateway),
        request,
        selection,
        move |kind| port.data_permit(kind),
    )
    .await
}

async fn handle_prepared<P: DataAccessPort>(
    port: Arc<P>,
    gateway: Option<Arc<Gateway>>,
    mut request: Request<Body>,
    (kind, file_request): (DataKind, bool),
    authorize: impl FnOnce(DataKind) -> Result<(P::Permit, Option<String>), PortError> + Send + 'static,
) -> Response<Body> {
    let control = control_request(&request);
    let request_lease = match admit_request(gateway.as_deref(), control) {
        Ok(lease) => lease,
        Err(status) => return response(status),
    };
    let (mut permit, guest_credential) =
        match authorize_data(Arc::clone(&request_lease), kind, authorize).await {
            Ok(permit) => permit,
            Err(status) => return response(status),
        };
    if let Some(secret) = guest_credential
        && replace_guest_credential(&mut request, &secret).is_err()
    {
        return response(StatusCode::UNAUTHORIZED);
    }
    let destination = permit.target();
    let lease = match &gateway {
        Some(gateway) => match gateway.acquire((destination.vm.0, destination.generation), control)
        {
            Ok(lease) => Some(lease),
            Err(status) => return response(status),
        },
        None => None,
    };
    let limits = gateway.as_ref().map(|gateway| &gateway.limits);
    let request = match until_revoked(
        permit.revocation(),
        prepare_body(request, file_request, limits),
    )
    .await
    {
        Some(Ok(request)) => request,
        Some(Err(rejection)) => return rejection,
        None => return response(StatusCode::UNAUTHORIZED),
    };
    let (returned_permit, mut child) =
        match launch_data_route(Arc::clone(&port), permit, control, request_lease).await {
            Ok(value) => value,
            Err(status) => return response(status),
        };
    permit = returned_permit;
    let mut relay_exit = drain::RelayExit::new(permit.drain().clone());
    let Ok((read, write)) = child.take_pipes() else {
        return response(StatusCode::BAD_GATEWAY);
    };
    let tunnel = StdioTunnel { read, write };
    let tunnel = WriteIdle::new(
        tunnel,
        limits.map(|limits| Duration::from_secs(limits.write_idle_seconds)),
    );
    let mut builder = http1::Builder::new();
    if limits.is_some() {
        builder.max_buf_size(64 * 1024);
    }
    let handshake =
        until_revoked(permit.revocation(), builder.handshake(TokioIo::new(tunnel))).await;
    let Some(Ok((mut sender, connection))) = handshake else {
        relay_exit.stop(&mut child).await;
        return response(if handshake.is_none() {
            StatusCode::UNAUTHORIZED
        } else {
            StatusCode::BAD_GATEWAY
        });
    };
    let connection_lease = lease.clone();
    let mut connection_revocation = permit.revocation().clone();
    tokio::spawn(async move {
        let _ = until_revoked(&mut connection_revocation, connection).await;
        relay_exit.stop(&mut child).await;
        drop(connection_lease);
    });
    let response = until_revoked(
        permit.revocation(),
        forward_request(&mut sender, request, limits),
    )
    .await
    .unwrap_or_else(|| response(StatusCode::UNAUTHORIZED));
    let response = drain::DrainBody::wrap(response, permit.drain().clone());
    match lease {
        Some(lease) => hold_response(response, lease),
        None => response,
    }
}

fn replace_guest_credential(request: &mut Request<Body>, secret: &str) -> Result<(), ()> {
    let value = axum::http::HeaderValue::from_str(secret).map_err(|_| ())?;
    request.headers_mut().insert("x-access-token", value);
    Ok(())
}

fn admit_request(
    gateway: Option<&Gateway>,
    control: bool,
) -> Result<Arc<RequestLease>, StatusCode> {
    // Even a syntactically valid unknown token consumes a pre-auth slot.
    // Production shares this budget with management and reconciliation;
    // standalone transport fixtures are finite too.
    static STANDALONE_REQUESTS: std::sync::OnceLock<Arc<RequestAdmission>> =
        std::sync::OnceLock::new();
    let requests = gateway.map_or_else(
        || STANDALONE_REQUESTS.get_or_init(|| Arc::new(RequestAdmission::default())),
        |gateway| &gateway.requests,
    );
    let class = if control {
        RequestClass::Control
    } else {
        RequestClass::Ordinary
    };
    requests
        .try_acquire(class)
        .ok_or(StatusCode::TOO_MANY_REQUESTS)
}

async fn authorize_data<P: DataAuthorization>(
    lease: Arc<RequestLease>,
    kind: DataKind,
    authorize: impl FnOnce(DataKind) -> Result<(P, Option<String>), PortError> + Send + 'static,
) -> Result<(P, Option<String>), StatusCode> {
    match tokio::task::spawn_blocking(move || {
        let _lease = lease;
        authorize(kind)
    })
    .await
    {
        Ok(Ok(value)) => Ok(value),
        Ok(Err(PortError::NotFound | PortError::Invalid)) => Err(StatusCode::UNAUTHORIZED),
        Ok(Err(_)) | Err(_) => Err(StatusCode::SERVICE_UNAVAILABLE),
    }
}

async fn launch_data_route<P: DataAccessPort>(
    port: Arc<P>,
    permit: P::Permit,
    control: bool,
    request_lease: Arc<RequestLease>,
) -> Result<(P::Permit, drain::RelayProcess), StatusCode> {
    let (permit, launched) = tokio::task::spawn_blocking(move || {
        let _lease = request_lease;
        let result = port.spawn_data_relay(&permit, control);
        (permit, result)
    })
    .await
    .map_err(|_| StatusCode::BAD_GATEWAY)?;
    let child = match launched {
        Ok(child) => drain::RelayProcess::new(child),
        Err(PortError::NotFound | PortError::Invalid) => return Err(StatusCode::UNAUTHORIZED),
        Err(PortError::QuotaExceeded) => return Err(StatusCode::TOO_MANY_REQUESTS),
        Err(_) => return Err(StatusCode::SERVICE_UNAVAILABLE),
    };
    Ok((permit, child))
}

/// One process per trusted, namespace-selected data connection. It transports
/// bytes only; no user-controlled target address or host path is accepted.
///
/// # Errors
/// Fails when envd is unreachable or the caller disconnects.
pub async fn relay() -> Result<(), Box<dyn Error>> {
    let stream = tokio::net::TcpStream::connect(ENVD_ADDRESS).await?;
    let (reader, writer) = stream.into_split();
    tunnel::relay(tokio::io::stdin(), tokio::io::stdout(), reader, writer).await?;
    Ok(())
}

#[cfg(test)]
mod port_tests;

#[cfg(all(target_os = "linux", test, feature = "sdk-contract-tests"))]
mod sdk_tests;

#[cfg(test)]
fn fixture_limits() -> TransferLimits {
    // Test data only; formal deployment still supplies and validates all limits.
    TransferLimits {
        file_upload_bytes: 64 * 1024 * 1024,
        file_download_bytes: 96 * 1024 * 1024,
        command_input_bytes: sandproof_data_transport::PROCESS_REQUEST_LIMIT,
        rpc_request_bytes: 64 * 1024,
        rpc_response_bytes: 1024 * 1024,
        max_transfers: 8,
        max_transfers_per_vm: 4,
        control_reserve: 1,
        request_idle_seconds: 5,
        file_idle_seconds: 10,
        write_idle_seconds: 5,
    }
}
