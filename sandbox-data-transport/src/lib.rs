#![forbid(unsafe_code)]
//! Shared, bounded HTTP body and byte transport. This layer does not authorize
//! users, resolve sandboxes, select a host, open a namespace, or launch a child.
//! Callers must complete their own durable authorization before forwarding.

use axum::{
    body::{Body, to_bytes},
    http::{Method, Request, Response, StatusCode, Version, header},
};
use http_body_util::{BodyStream, Limited};
use hyper::{body::Body as _, client::conn::http1::SendRequest};
use std::{
    error::Error,
    io,
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
    time::Duration,
};
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};

mod admission;
mod gzip_upload;
pub use gzip_upload::BodyWorkOwner;
mod idle;
mod limits;
mod request;
mod revocation;
pub mod tunnel;
mod upload;
pub use admission::{TransferBudget, TransferLease, control_request, hold_response};
use idle::{Activity, IdleBody};
pub use idle::{GatewayListener, WriteIdle};
pub use limits::TransferLimits;
pub use request::{access_token, request_kind};
pub use revocation::until_revoked;
use upload::{UploadBody, UploadProgress};

pub const FILE_REQUEST_LIMIT: usize = 8 * 1024 * 1024;
pub const PROCESS_REQUEST_LIMIT: usize = 1024 * 1024;
pub const RESPONSE_LIMIT: usize = 32 * 1024 * 1024;
const ENVD_ADDRESS: &str = "172.16.0.2:49983";

#[must_use]
pub fn response(status: StatusCode) -> Response<Body> {
    let mut response = Response::new(Body::empty());
    *response.status_mut() = status;
    response
}

fn contains_nul_escape(bytes: &[u8]) -> bool {
    bytes.contains(&0)
        || bytes
            .windows(6)
            .any(|window| window.eq_ignore_ascii_case(br"\u0000"))
}

// Shared by the authenticated production path and transport regression tests.
// Tests replace only the envd transport; the actual limits and framing remain.
/// Apply the original streaming input/framing policy to an authorized request.
/// This function does not choose or authorize its downstream destination.
/// # Errors
/// Returns a status-only rejection for unsupported framing/encoding, malformed
/// file metadata, input limits or producer idle deadlines.
pub async fn prepare_body(
    mut request: Request<Body>,
    file_request: bool,
    limits: Option<&TransferLimits>,
) -> Result<Request<Body>, Response<Body>> {
    // The fixed SDK uses HTTP/1.1. HTTP/1.0 cannot distinguish a truncated
    // unknown-length stream from success; do not execute it on that basis.
    if request.version() != Version::HTTP_11 {
        return Err(response(StatusCode::HTTP_VERSION_NOT_SUPPORTED));
    }
    let gzip_upload = limits.is_some()
        && request.uri().path() == "/files"
        && request.method() == Method::POST
        && gzip_upload_encoding(request.headers());
    if limits.is_some() && !identity_encoding(request.headers()) && !gzip_upload {
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
        let (mut parts, mut body) = request.into_parts();
        if body.size_hint().lower() > limits.file_upload_bytes as u64 {
            return Err(response(StatusCode::PAYLOAD_TOO_LARGE));
        }
        if gzip_upload {
            let owner = parts.extensions.get::<BodyWorkOwner>().cloned();
            let (decoded, progress) = gzip_upload::open(body, limits, owner)
                .await
                .map_err(response)?;
            body = decoded;
            // These bytes really are decoded, unlike silently stripping an
            // encoding from an unchanged request. Never retain encoded length.
            parts.headers.remove(header::CONTENT_ENCODING);
            parts.headers.remove(header::CONTENT_LENGTH);
            parts.headers.remove(header::TRANSFER_ENCODING);
            parts.extensions.insert(progress);
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
    // Compressed command/RPC envelopes remain outside this file-only change.
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

fn gzip_upload_encoding(headers: &axum::http::HeaderMap) -> bool {
    let mut encodings = headers.get_all(header::CONTENT_ENCODING).iter();
    let gzip = encodings.next().is_some_and(|value| {
        value
            .to_str()
            .is_ok_and(|value| value.trim().eq_ignore_ascii_case("gzip"))
    }) && encodings.next().is_none();
    gzip && ["connect-content-encoding", "grpc-encoding"]
        .iter()
        .all(|name| {
            headers.get_all(*name).iter().all(|value| {
                value
                    .to_str()
                    .is_ok_and(|value| value.trim().eq_ignore_ascii_case("identity"))
            })
        })
}

pub async fn forward_request(
    sender: &mut SendRequest<Body>,
    mut request: Request<Body>,
    limits: Option<&TransferLimits>,
) -> Response<Body> {
    let downstream_version = request.version();
    let upload = request.extensions().get::<UploadProgress>().cloned();
    let gzip = request.extensions().get::<gzip_upload::Progress>().cloned();
    let activity = request.extensions().get::<Activity>().cloned();
    let file_response = request.uri().path() == "/files"
        || request.uri().path().starts_with("/filesystem.Filesystem/");
    let file_download = request.uri().path() == "/files" && request.method() == Method::GET;
    // Start/Connect carry a potentially long guest output stream. Backpressure,
    // connection admission and revocation bound host residency, not total bytes.
    let command_stream = matches!(
        request.uri().path(),
        "/process.Process/Start" | "/process.Process/Connect"
    );
    let response_limit = limits.map_or(RESPONSE_LIMIT, |limits| {
        if file_download {
            limits.file_download_bytes
        } else {
            limits.rpc_response_bytes
        }
    });
    if limits.is_some() {
        // This is a valid HTTP content negotiation choice, including when a
        // SDK asks for gzip: identity bytes make the enforced budget unambiguous.
        request.headers_mut().insert(
            header::ACCEPT_ENCODING,
            axum::http::HeaderValue::from_static("identity"),
        );
        request.headers_mut().insert(
            "connect-accept-encoding",
            axum::http::HeaderValue::from_static("identity"),
        );
    }
    request.headers_mut().remove("x-api-key");
    request.headers_mut().insert(
        header::CONNECTION,
        axum::http::HeaderValue::from_static("close"),
    );
    request.headers_mut().remove(header::TRANSFER_ENCODING);
    request.headers_mut().remove(header::CONTENT_LENGTH);
    if let Some(length) = upload.as_ref().and_then(|progress| progress.length) {
        request
            .headers_mut()
            .insert(header::CONTENT_LENGTH, length.into());
    }
    request.headers_mut().insert(
        header::HOST,
        axum::http::HeaderValue::from_static(ENVD_ADDRESS),
    );
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
    if let Some(status) = gzip.as_ref().and_then(gzip_upload::Progress::status) {
        return response(status);
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
                let body = if command_stream {
                    Body::new(BodyStream::new(body))
                } else {
                    Body::new(BodyStream::new(Limited::new(body, response_limit)))
                };
                match limits.filter(|_| file_response) {
                    Some(limits) => IdleBody::wrap(body, limits.file_idle_seconds, Activity::new()),
                    None => body,
                }
            })
        }
        Err(_) => response(StatusCode::BAD_GATEWAY),
    }
}

#[cfg(test)]
mod tests;
