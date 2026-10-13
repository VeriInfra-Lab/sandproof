//! Fixed download error envelopes, bounded before allocating their payload.
// Messages/envelope from e2b-dev/runtime f455fd58fa3f289e78ce1de690c35968d8d29322,
// packages/envd/internal/api/{download,error,api.gen}.go (Apache-2.0).
// License: ../../../../sandbox-guest-runtime/src/file_encoding/E2B-LICENSE.
use super::{DownloadError, IdentityLimits, IdentityRequest};
use crate::file_download::{BodyPlan, Error, ResponsePlan, header_budget, serve_error};
use axum::http::{HeaderMap, HeaderValue, StatusCode};
use sandproof_guest_runtime::file_download::LinuxErrno;

pub(super) fn json(
    error: DownloadError,
    request: &IdentityRequest,
    limits: IdentityLimits,
) -> Result<ResponsePlan, DownloadError> {
    let path = request.disposition_path.as_deref();
    let mut headers = request.response.clone();
    let mut parts: Vec<&[u8]> = Vec::with_capacity(8);
    let errno;
    let status = match error {
        DownloadError::SourceNotFound | DownloadError::StatIo(LinuxErrno(2)) => {
            parts.extend([
                b"path '".as_slice(),
                path.ok_or(error)?,
                b"' does not exist",
            ]);
            StatusCode::NOT_FOUND
        }
        DownloadError::Directory => {
            parts.extend([
                b"path '".as_slice(),
                path.ok_or(error)?,
                b"' is a directory",
            ]);
            StatusCode::BAD_REQUEST
        }
        DownloadError::StatIo(number) | DownloadError::OpenIo(number) => {
            errno = number.message();
            let path = path.ok_or(error)?;
            if matches!(error, DownloadError::StatIo(_)) {
                parts.extend([
                    b"error checking if path exists '".as_slice(),
                    path,
                    b"': stat ",
                    path,
                ]);
            } else {
                parts.extend([b"error opening file '".as_slice(), path, b"': open ", path]);
            }
            parts.extend([b": ".as_slice(), errno.as_bytes()]);
            StatusCode::INTERNAL_SERVER_ERROR
        }
        DownloadError::Negotiation(negotiation)
            if negotiation.cause == crate::file_encoding::Error::NotAcceptable =>
        {
            if negotiation.vary_accept_encoding {
                headers.insert("vary", HeaderValue::from_static("Accept-Encoding"));
                parts.push(b"identity encoding not acceptable for Range or conditional request");
            } else {
                parts.push(b"error parsing Accept-Encoding: no acceptable encoding found, supported: [gzip]");
            }
            StatusCode::NOT_ACCEPTABLE
        }
        _ => return Err(error),
    };
    let bytes = payload(&parts, Some(status), limits.plan.wire_bytes)?;
    headers.insert(
        "content-type",
        HeaderValue::from_static("application/json; charset=utf-8"),
    );
    headers.insert(
        "x-content-type-options",
        HeaderValue::from_static("nosniff"),
    );
    header_budget(&headers, limits.plan.response_header_bytes)?;
    Ok(ResponsePlan {
        status,
        headers,
        body: BodyPlan::Error(bytes.into()),
        required_seek: None,
        file_bytes: 0,
    })
}

pub(super) fn range(
    errno: LinuxErrno,
    path: Option<&[u8]>,
    headers: HeaderMap,
    limits: IdentityLimits,
) -> Result<ResponsePlan, DownloadError> {
    let path = path.ok_or(DownloadError::RangeSeekIo(errno))?;
    let message = errno.message();
    let bytes = payload(
        &[b"seek ", path, b": ", message.as_bytes(), b"\n"],
        None,
        limits.plan.wire_bytes,
    )?;
    Ok(serve_error(
        headers,
        StatusCode::RANGE_NOT_SATISFIABLE,
        bytes.into(),
        limits.plan,
    )?)
}

// Match encoding/json.Encoder's default escaping, field order and newline.
// Count with the same emitter first; never allocate an oversized escaped path.
fn payload(parts: &[&[u8]], json: Option<StatusCode>, limit: u64) -> Result<Vec<u8>, Error> {
    let emit = |put: &mut crate::go_json::Sink<'_>| -> Result<(), Error> {
        if let Some(status) = json {
            put(b"{\"code\":")?;
            put(status.as_str().as_bytes())?;
            put(b",\"message\":\"")?;
        }
        for part in parts {
            if json.is_none() {
                put(part)?;
                continue;
            }
            crate::go_json::string_content(part, put)?;
        }
        if json.is_some() {
            put(b"\"}\n")?;
        }
        Ok(())
    };
    crate::go_json::bounded(limit, emit)
}
