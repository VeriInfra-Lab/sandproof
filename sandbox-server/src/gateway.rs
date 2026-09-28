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
use sandproof_api::{
    PortError,
    managed::{DataKind, ManagedPort},
};
use tokio::{
    io::{AsyncRead, AsyncWrite, AsyncWriteExt, ReadBuf},
    process::{ChildStdin, ChildStdout, Command},
};

use crate::linux::NodeHost;

mod admission;
mod idle;
mod limits;
mod upload;
pub(crate) use admission::Gateway;
use admission::{LeasedBody, control_request};
pub(crate) use idle::GatewayListener;
use idle::{Activity, IdleBody, WriteIdle};
pub(crate) use limits::TransferLimits;
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

fn spawn_relay(namespace_name: &str) -> io::Result<tokio::process::Child> {
    Command::new("/usr/bin/ip")
        .args(["netns", "exec", namespace_name])
        .arg(std::env::current_exe()?)
        .arg("relay")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true)
        .spawn()
}

/// SDK data requests carry only the sandbox's envd bearer token. It is
/// resolved to one running sandbox, and the core checks effective Exec or
/// FileRead/FileWrite permission before launching a namespace-local relay.
pub async fn handle(
    port: Arc<ManagedPort<NodeHost>>,
    namespace: String,
    gateway: Option<Arc<Gateway>>,
    request: Request<Body>,
) -> Response<Body> {
    let path = request.uri().path().to_owned();
    let Some((kind, file_request)) = operation(request.method(), &path) else {
        return response(StatusCode::NOT_FOUND);
    };
    let raw_url = request
        .uri()
        .path_and_query()
        .map_or("", |value| value.as_str());
    if raw_url.len() > URL_LIMIT {
        return response(StatusCode::URI_TOO_LONG);
    }
    if file_request
        && (percent_decode_str(raw_url).decode_utf8().is_err()
            || percent_decode_str(raw_url)
                .decode_utf8()
                .is_ok_and(|decoded| decoded.contains('\0')))
    {
        return response(StatusCode::BAD_REQUEST);
    }
    let mut tokens = request.headers().get_all("x-access-token").iter();
    let Some(token) = tokens.next() else {
        return response(StatusCode::UNAUTHORIZED);
    };
    if tokens.next().is_some() {
        return response(StatusCode::UNAUTHORIZED);
    }
    let Ok(token) = token.to_str() else {
        return response(StatusCode::UNAUTHORIZED);
    };
    let token = token.to_owned();
    let destination =
        match tokio::task::spawn_blocking(move || port.authorize_data(&token, kind)).await {
            Ok(Ok(value)) => value,
            Ok(Err(PortError::NotFound | PortError::Invalid)) => {
                return response(StatusCode::UNAUTHORIZED);
            }
            Ok(Err(_)) | Err(_) => return response(StatusCode::SERVICE_UNAVAILABLE),
        };
    let lease = match &gateway {
        Some(gateway) => match gateway.acquire(
            (destination.vm.0, destination.generation),
            control_request(&request),
        ) {
            Ok(lease) => Some(lease),
            Err(status) => return response(status),
        },
        None => None,
    };
    let limits = gateway.as_ref().map(|gateway| &gateway.limits);
    let request = match prepare_body(request, file_request, limits).await {
        Ok(request) => request,
        Err(rejection) => return rejection,
    };
    let namespace_name = format!("vm-{namespace}-{}", destination.vm.0);
    let Ok(mut child) = spawn_relay(&namespace_name) else {
        return response(StatusCode::BAD_GATEWAY);
    };
    let tunnel = StdioTunnel {
        read: child.stdout.take().expect("piped stdout"),
        write: child.stdin.take().expect("piped stdin"),
    };
    let tunnel = WriteIdle::new(
        tunnel,
        limits.map(|limits| Duration::from_secs(limits.write_idle_seconds)),
    );
    let mut builder = http1::Builder::new();
    if limits.is_some() {
        builder.max_buf_size(64 * 1024);
    }
    let Ok((mut sender, connection)) = builder.handshake(TokioIo::new(tunnel)).await else {
        let _ = child.kill().await;
        return response(StatusCode::BAD_GATEWAY);
    };
    let connection_lease = lease.clone();
    tokio::spawn(async move {
        let _ = connection.await;
        let _ = child.kill().await;
        drop(connection_lease);
    });
    let response = forward_request(&mut sender, request, limits).await;
    match lease {
        Some(lease) => LeasedBody::wrap(response, lease),
        None => response,
    }
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
pub async fn relay() -> Result<(), Box<dyn Error>> {
    let stream = tokio::net::TcpStream::connect(ENVD_ADDRESS).await?;
    let (mut reader, mut writer) = stream.into_split();
    let mut input = tokio::io::stdin();
    let mut output = tokio::io::stdout();
    let upstream = async {
        tokio::io::copy(&mut input, &mut writer).await?;
        writer.shutdown().await?;
        Ok::<(), io::Error>(())
    };
    let downstream = async {
        tokio::io::copy(&mut reader, &mut output).await?;
        output.flush().await?;
        Ok::<(), io::Error>(())
    };
    tokio::try_join!(upstream, downstream)?;
    Ok(())
}

#[cfg(test)]
mod tests;

#[cfg(all(test, feature = "sdk-contract-tests"))]
mod sdk_tests;
