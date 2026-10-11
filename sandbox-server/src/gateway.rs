//! Fail-closed envd gateway. The user sees neither the VM network namespace
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
    body::{Body, to_bytes},
    http::{Method, Request, Response, StatusCode, Version, header},
};
use http_body_util::{BodyStream, Limited};
use hyper::{
    body::Body as _,
    client::conn::http1::{self, SendRequest},
};
use hyper_util::rt::TokioIo;
use percent_encoding::percent_decode_str;
use sandproof_api::admission::{RequestAdmission, RequestClass, RequestLease};
use sandproof_controller_service::application::{
    ApplicationError as PortError, DataAccessPort, DataAuthorization, DataKind,
};
use tokio::{
    io::{AsyncRead, AsyncWrite, ReadBuf},
    process::{ChildStdin, ChildStdout},
};

mod admission;
mod drain;
mod idle;
mod limits;
mod revocation;
mod tunnel;
mod upload;
pub(crate) use admission::Gateway;
use admission::{LeasedBody, control_request};
pub(crate) use idle::GatewayListener;
use idle::{Activity, IdleBody, WriteIdle};
pub(crate) use limits::TransferLimits;
use revocation::until_revoked;
use upload::{UploadBody, UploadProgress};

const FILE_REQUEST_LIMIT: usize = 8 * 1024 * 1024;
const PROCESS_REQUEST_LIMIT: usize = 1024 * 1024;
const RESPONSE_LIMIT: usize = 32 * 1024 * 1024;
const URL_LIMIT: usize = 4096;
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

fn response(status: StatusCode) -> Response<Body> {
    Response::builder()
        .status(status)
        .body(Body::empty())
        .expect("static status")
}

fn operation(method: &Method, path: &str) -> Option<(DataKind, bool)> {
    if path == "/health" && method == Method::GET {
        return Some((DataKind::Exec, false));
    }
    if path.starts_with("/process.Process/") && method == Method::POST {
        return Some((DataKind::Exec, false));
    }
    if path == "/files" {
        return match *method {
            Method::GET => Some((DataKind::FileRead, true)),
            Method::POST | Method::DELETE => Some((DataKind::FileWrite, true)),
            _ => None,
        };
    }
    if let Some(name) = path.strip_prefix("/filesystem.Filesystem/") {
        if method != Method::POST {
            return None;
        }
        let kind = match name {
            "ListDir" | "Stat" | "ReadFile" | "GetInfo" => DataKind::FileRead,
            "MakeDir" | "Remove" | "Move" | "WriteFile" | "Copy" => DataKind::FileWrite,
            _ => return None,
        };
        return Some((kind, true));
    }
    None
}

fn contains_nul_escape(bytes: &[u8]) -> bool {
    bytes.contains(&0)
        || bytes
            .windows(6)
            .any(|window| window.eq_ignore_ascii_case(br"\u0000"))
}

fn access_token(headers: &axum::http::HeaderMap) -> Result<String, StatusCode> {
    let mut tokens = headers.get_all("x-access-token").iter();
    let token = tokens.next().ok_or(StatusCode::UNAUTHORIZED)?;
    if tokens.next().is_some() {
        return Err(StatusCode::UNAUTHORIZED);
    }
    token
        .to_str()
        .map(str::to_owned)
        .map_err(|_| StatusCode::UNAUTHORIZED)
}

fn request_kind(request: &Request<Body>) -> Result<(DataKind, bool), StatusCode> {
    let (kind, file_request) =
        operation(request.method(), request.uri().path()).ok_or(StatusCode::NOT_FOUND)?;
    let raw_url = request
        .uri()
        .path_and_query()
        .map_or("", |value| value.as_str());
    if raw_url.len() > URL_LIMIT {
        return Err(StatusCode::URI_TOO_LONG);
    }
    if file_request
        && (percent_decode_str(raw_url).decode_utf8().is_err()
            || percent_decode_str(raw_url)
                .decode_utf8()
                .is_ok_and(|decoded| decoded.contains('\0')))
    {
        return Err(StatusCode::BAD_REQUEST);
    }
    Ok((kind, file_request))
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
    let control = control_request(&request);
    let request_lease = match admit_request(gateway.as_deref(), control) {
        Ok(lease) => lease,
        Err(status) => return response(status),
    };
    let mut permit =
        match authorize_data(Arc::clone(&port), Arc::clone(&request_lease), token, kind).await {
            Ok(permit) => permit,
            Err(status) => return response(status),
        };
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
        Some(lease) => LeasedBody::wrap(response, lease),
        None => response,
    }
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

async fn authorize_data<P: DataAccessPort>(
    port: Arc<P>,
    lease: Arc<RequestLease>,
    token: String,
    kind: DataKind,
) -> Result<P::Permit, StatusCode> {
    match tokio::task::spawn_blocking(move || {
        let _lease = lease;
        port.data_permit(&token, kind)
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

// Shared by the authenticated production path and transport regression tests.
// Tests replace only the envd transport; the actual limits and framing remain.
async fn prepare_body(
    mut request: Request<Body>,
    file_request: bool,
    limits: Option<&TransferLimits>,
) -> Result<Request<Body>, Response<Body>> {
    // The fixed SDK uses HTTP/1.1. HTTP/1.0 cannot distinguish a truncated
    // unknown-length stream from success; do not execute it on that basis.
    if request.version() != Version::HTTP_11 {
        return Err(response(StatusCode::HTTP_VERSION_NOT_SUPPORTED));
    }
    if limits.is_some() && !identity_encoding(request.headers()) {
        return Err(response(StatusCode::UNSUPPORTED_MEDIA_TYPE));
    }
    if let Some(limits) = limits {
        let (mut parts, body) = request.into_parts();
        let activity = Activity::new();
        parts.extensions.insert(activity.clone());
        request = Request::from_parts(
            parts,
            IdleBody::wrap(body, limits.request_idle_seconds, activity),
        );
    }
    if let Some(limits) = limits
        && request.uri().path() == "/files"
        && request.method() == Method::POST
    {
        let (mut parts, body) = request.into_parts();
        if body.size_hint().lower() > limits.file_upload_bytes as u64 {
            return Err(response(StatusCode::PAYLOAD_TOO_LARGE));
        }
        let (body, progress) = UploadBody::new(body, limits.file_upload_bytes);
        parts.extensions.insert(progress);
        request = Request::from_parts(parts, Body::new(body));
    } else if file_request {
        let (parts, body) = request.into_parts();
        let budget = limits.map_or(FILE_REQUEST_LIMIT, |value| value.rpc_request_bytes);
        let Ok(bytes) = to_bytes(body, budget).await else {
            let status = if parts
                .extensions
                .get::<Activity>()
                .is_some_and(Activity::timed_out)
            {
                StatusCode::REQUEST_TIMEOUT
            } else {
                StatusCode::PAYLOAD_TOO_LARGE
            };
            return Err(response(status));
        };
        if parts.uri.path().starts_with("/filesystem.Filesystem/") && contains_nul_escape(&bytes) {
            return Err(response(StatusCode::BAD_REQUEST));
        }
        request = Request::from_parts(parts, Body::from(bytes));
    } else {
        // Process RPCs may be bidirectional. Preserve streaming while bounding
        // client-to-guest input; buffering the whole request can deadlock Start.
        let (parts, body) = request.into_parts();
        request = Request::from_parts(
            parts,
            // Limited clamps an exact size hint to the budget. Do not let
            // HTTP turn that hint into a shorter successful Content-Length:
            // the over-limit frame must still be polled and fail the stream.
            Body::new(BodyStream::new(Limited::new(
                body,
                limits.map_or(PROCESS_REQUEST_LIMIT, |value| value.command_input_bytes),
            ))),
        );
    }
    Ok(request)
}

fn identity_encoding(headers: &axum::http::HeaderMap) -> bool {
    // envd can decompress both HTTP uploads and Connect streams. Until a
    // separate decoded-byte budget exists, neither encoding is supported.
    [
        "content-encoding",
        "connect-content-encoding",
        "grpc-encoding",
    ]
    .iter()
    .all(|name| {
        headers.get_all(*name).iter().all(|value| {
            value
                .to_str()
                .is_ok_and(|value| value.trim().eq_ignore_ascii_case("identity"))
        })
    })
}

async fn forward_request(
    sender: &mut SendRequest<Body>,
    mut request: Request<Body>,
    limits: Option<&TransferLimits>,
) -> Response<Body> {
    let downstream_version = request.version();
    let upload = request.extensions().get::<UploadProgress>().cloned();
    let activity = request.extensions().get::<Activity>().cloned();
    let file_response = request.uri().path() == "/files"
        || request.uri().path().starts_with("/filesystem.Filesystem/");
    let file_download = request.uri().path() == "/files" && request.method() == Method::GET;
    let response_limit = limits.map_or(RESPONSE_LIMIT, |limits| {
        if file_download {
            limits.file_download_bytes
        } else if request.uri().path().starts_with("/process.Process/") {
            limits.command_output_bytes
        } else {
            limits.rpc_response_bytes
        }
    });
    if limits.is_some() {
        // This is a valid HTTP content negotiation choice, including when a
        // SDK asks for gzip: identity bytes make the enforced budget unambiguous.
        request
            .headers_mut()
            .insert(header::ACCEPT_ENCODING, "identity".parse().unwrap());
        request
            .headers_mut()
            .insert("connect-accept-encoding", "identity".parse().unwrap());
    }
    request.headers_mut().remove("x-api-key");
    request.headers_mut().insert(
        header::CONNECTION,
        "close".parse().expect("static connection"),
    );
    request.headers_mut().remove(header::TRANSFER_ENCODING);
    request.headers_mut().remove(header::CONTENT_LENGTH);
    if let Some(length) = upload.as_ref().and_then(|progress| progress.length) {
        request
            .headers_mut()
            .insert(header::CONTENT_LENGTH, length.into());
    }
    request
        .headers_mut()
        .insert(header::HOST, ENVD_ADDRESS.parse().expect("static host"));
    let path_and_query = request
        .uri()
        .path_and_query()
        .map_or("/", |value| value.as_str());
    let Ok(uri) = path_and_query.parse() else {
        return response(StatusCode::BAD_REQUEST);
    };
    *request.uri_mut() = uri;
    let result = if let (Some(limits), Some(activity)) =
        (limits.filter(|_| file_response), activity.as_ref())
    {
        tokio::select! {
            biased;
            result = sender.send_request(request) => result,
            () = activity.stalled(Duration::from_secs(limits.file_idle_seconds)) => {
                return response(if activity.timed_out() { StatusCode::REQUEST_TIMEOUT } else { StatusCode::GATEWAY_TIMEOUT });
            }
        }
    } else {
        sender.send_request(request).await
    };
    if activity.as_ref().is_some_and(Activity::timed_out) {
        return response(StatusCode::REQUEST_TIMEOUT);
    }
    if upload.as_ref().is_some_and(UploadProgress::over_limit) {
        return response(StatusCode::PAYLOAD_TOO_LARGE);
    }
    match result {
        Ok(mut upstream) => {
            if upstream.status().is_success()
                && upload.as_ref().is_some_and(|progress| !progress.complete())
            {
                // An early success is not evidence that the final bytes
                // arrived. Partial writes remain possible/unknown.
                return response(StatusCode::BAD_GATEWAY);
            }
            if limits.is_some() && !identity_encoding(upstream.headers()) {
                return response(StatusCode::BAD_GATEWAY);
            }
            // The two HTTP connections negotiate versions independently. An
            // upstream HTTP/1.0 response must not downgrade an HTTP/1.1 client
            // to close-delimited output: EOF would hide a stream limit error.
            *upstream.version_mut() = downstream_version;
            // Keep the upstream Content-Length, if present, but erase the
            // clamped hint. Otherwise it conflicts with the original header
            // (a Hyper assertion), or can disguise truncation as completion.
            // BodyStream preserves data/trailers/errors without buffering.
            upstream.map(|body| {
                let body = Body::new(BodyStream::new(Limited::new(body, response_limit)));
                match limits.filter(|_| file_response) {
                    Some(limits) => IdleBody::wrap(body, limits.file_idle_seconds, Activity::new()),
                    None => body,
                }
            })
        }
        Err(_) => response(StatusCode::BAD_GATEWAY),
    }
}

/// One process per trusted, namespace-selected data connection. It transports
/// bytes only; no user-controlled target address or host path is accepted.
///
/// # Errors
/// Fails when envd is unreachable or the caller disconnects.
pub async fn relay(bounded: bool) -> Result<(), Box<dyn Error>> {
    let stream = tokio::net::TcpStream::connect(ENVD_ADDRESS).await?;
    let (reader, writer) = stream.into_split();
    tunnel::relay(
        bounded,
        tokio::io::stdin(),
        tokio::io::stdout(),
        reader,
        writer,
    )
    .await?;
    Ok(())
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod port_tests;

#[cfg(all(test, feature = "sdk-contract-tests"))]
mod sdk_tests;
