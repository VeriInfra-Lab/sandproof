//! Bounded replies for observed Connect method and header failures. No arbitrary
//! diagnostics, codec policy or command-operation errors are mapped.
use super::Error as SelectionError;
#[cfg(any(target_os = "linux", test))]
use super::{Encoding, ResponseFormat};
#[cfg(any(target_os = "linux", test))]
use crate::finite_body;
use crate::owned_body::{self, Owner};
#[cfg(any(target_os = "linux", test))]
use axum::{
    http::{HeaderValue, StatusCode},
    response::Response,
};
use std::{fmt, sync::Arc};
use tokio::time::Instant;

#[cfg(test)]
mod tests;
#[cfg(any(target_os = "linux", test))]
mod wire;

/// One original admitted request covers the bounded source diagnostic, raw
/// payload, optional compressed frame, codec workspace and transmission chunk.
#[derive(Clone, Copy, Debug)]
pub struct Limits {
    /// Maximum complete wire payload, including the stream's five-byte header.
    /// The uncompressed payload must independently fit this same bound.
    pub wire_bytes: usize,
    pub chunk_bytes: usize,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    InvalidConfiguration,
    ResponseLimit,
    Encoding,
    DeadlineExceeded,
    Unavailable,
    UnclassifiedFault,
}
impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "command header response: {self:?}")
    }
}
impl std::error::Error for Error {}
impl owned_body::Failure for Error {
    fn deadline() -> Self {
        Self::DeadlineExceeded
    }
    fn unavailable() -> Self {
        Self::Unavailable
    }
}

struct Source {
    error: SelectionError,
    deadline: Instant,
    _owner: Owner,
}
/// Preparation failures retain both the original observation and request slot.
pub struct Failure {
    error: Error,
    source: Arc<Source>,
}
impl Failure {
    #[must_use]
    pub const fn error(&self) -> Error {
        self.error
    }
    #[must_use]
    pub fn deadline(&self) -> Instant {
        self.source.deadline
    }
    #[must_use]
    pub fn selection_error(&self) -> &SelectionError {
        &self.source.error
    }
}
impl fmt::Debug for Failure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CommandHeaderResponseFailure")
            .field("error", &self.error)
            .finish_non_exhaustive()
    }
}

#[derive(Clone, Copy)]
#[cfg(any(target_os = "linux", test))]
struct Plan {
    method_head: Option<bool>,
    stream: bool,
    gzip: bool,
    content_type: &'static str,
    status: StatusCode,
}
#[cfg(any(target_os = "linux", test))]
impl Plan {
    fn select(error: &SelectionError, format: Option<ResponseFormat>) -> Result<Self, Error> {
        if let SelectionError::Method { head } = error {
            return Ok(Self {
                method_head: Some(*head),
                stream: false,
                gzip: false,
                content_type: "",
                status: StatusCode::METHOD_NOT_ALLOWED,
            });
        }
        let status = match error {
            SelectionError::Compression(_) => StatusCode::NOT_IMPLEMENTED,
            SelectionError::Version(_) | SelectionError::Timeout(_) => StatusCode::BAD_REQUEST,
            _ => return Err(Error::UnclassifiedFault),
        };
        let format = format.ok_or(Error::UnclassifiedFault)?;
        let stream = format.content_type().starts_with("application/connect+");
        Ok(Self {
            method_head: None,
            stream,
            gzip: stream && format.encoding() == Encoding::Gzip,
            // Unary Connect errors always use uncompressed application/json,
            // even when the successful response would use UTF-8/gzip.
            content_type: if stream {
                format.content_type()
            } else {
                "application/json"
            },
            status: if stream { StatusCode::OK } else { status },
        })
    }
}

#[cfg(any(target_os = "linux", test))]
pub(super) async fn send(
    error: SelectionError,
    format: Option<ResponseFormat>,
    limits: Limits,
    deadline: Instant,
    owner: Owner,
) -> Result<Response, Failure> {
    let source = Arc::new(Source {
        error,
        deadline,
        _owner: owner,
    });
    let result = send_inner(source.clone(), format, limits).await;
    result.map_err(|error| Failure { error, source })
}

#[cfg(any(target_os = "linux", test))]
async fn send_inner(
    source: Arc<Source>,
    format: Option<ResponseFormat>,
    limits: Limits,
) -> Result<Response, Error> {
    check_limits(limits)?;
    let plan = Plan::select(&source.error, format)?;
    let deadline = source.deadline;
    let payload = finite_body::prepare(deadline, source.clone(), move |cancelled| {
        wire::encode(&source.error, plan, limits, deadline, &cancelled)
    })
    .await?;
    response(payload, plan, limits, deadline)
}

#[cfg(any(target_os = "linux", test))]
fn check_limits(limits: Limits) -> Result<(), Error> {
    if limits.wire_bytes == 0
        || limits.chunk_bytes == 0
        || limits.wire_bytes > u32::MAX as usize
        || limits.wire_bytes > isize::MAX as usize / 16
        || limits.chunk_bytes > isize::MAX as usize / 16
    {
        return Err(Error::InvalidConfiguration);
    }
    Ok(())
}

/// Internal sender for already observed stream failures; the caller retains
/// the source on preparation errors, and supplies only bounded fixed JSON.
#[cfg(any(target_os = "linux", test))]
pub(super) async fn send_stream_error(
    format: ResponseFormat,
    limits: Limits,
    deadline: Instant,
    owner: Owner,
    encode: impl FnOnce(usize) -> Result<Vec<u8>, Error> + Send + 'static,
) -> Result<Response, Error> {
    check_limits(limits)?;
    if !format.content_type().starts_with("application/connect+") {
        return Err(Error::UnclassifiedFault);
    }
    let plan = Plan {
        method_head: None,
        stream: true,
        gzip: format.encoding() == Encoding::Gzip,
        content_type: format.content_type(),
        status: StatusCode::OK,
    };
    let payload = finite_body::prepare(deadline, owner, move |cancelled| {
        let limit = limits
            .wire_bytes
            .checked_sub(5)
            .ok_or(Error::ResponseLimit)?;
        let payload = encode(limit)?;
        wire::frame(&payload, plan.gzip, limits, deadline, &cancelled)
    })
    .await?;
    response(payload, plan, limits, deadline)
}

#[cfg(any(target_os = "linux", test))]
fn response(
    payload: finite_body::PreparedBody<Error>,
    plan: Plan,
    limits: Limits,
    deadline: Instant,
) -> Result<Response, Error> {
    let length = payload.length();
    let mut response = Response::new(payload.into_body(limits.chunk_bytes, deadline, false));
    *response.status_mut() = plan.status;
    let headers = response.headers_mut();
    if plan.method_head.is_some() {
        headers.insert("allow", HeaderValue::from_static("POST"));
    } else {
        headers.insert("content-type", HeaderValue::from_static(plan.content_type));
        headers.insert(
            if plan.stream {
                "connect-accept-encoding"
            } else {
                "accept-encoding"
            },
            HeaderValue::from_static("gzip"),
        );
    }
    if plan.gzip {
        headers.insert("connect-content-encoding", HeaderValue::from_static("gzip"));
    }
    // The original server flushes stream EOS, so never supplies a length. Its
    // unary writer only supplies an automatic length within the 2048-byte buffer.
    if !plan.stream && plan.method_head != Some(true) && length <= 2048 {
        headers.insert(
            "content-length",
            HeaderValue::from_str(&length.to_string()).map_err(|_| Error::Unavailable)?,
        );
    }
    Ok(response)
}
