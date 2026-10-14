//! Bounded replies for three observed Connect header failures. No arbitrary
//! diagnostics, method/codec policy or command-operation errors are mapped.
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
    stream: bool,
    gzip: bool,
    content_type: &'static str,
    status: StatusCode,
}
#[cfg(any(target_os = "linux", test))]
impl Plan {
    fn select(error: &SelectionError, format: Option<ResponseFormat>) -> Result<Self, Error> {
        let status = match error {
            SelectionError::Compression(_) => StatusCode::NOT_IMPLEMENTED,
            SelectionError::Version(_) | SelectionError::Timeout(_) => StatusCode::BAD_REQUEST,
            _ => return Err(Error::UnclassifiedFault),
        };
        let format = format.ok_or(Error::UnclassifiedFault)?;
        let stream = format.content_type().starts_with("application/connect+");
        Ok(Self {
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
    if limits.wire_bytes == 0
        || limits.chunk_bytes == 0
        || limits.wire_bytes > u32::MAX as usize
        || limits.wire_bytes > isize::MAX as usize / 16
        || limits.chunk_bytes > isize::MAX as usize / 16
    {
        return Err(Error::InvalidConfiguration);
    }
    let plan = Plan::select(&source.error, format)?;
    let deadline = source.deadline;
    let payload = finite_body::prepare(deadline, source.clone(), move |cancelled| {
        wire::encode(&source.error, plan, limits, deadline, &cancelled)
    })
    .await?;
    let length = payload.length();
    let mut response = Response::new(payload.into_body(limits.chunk_bytes, deadline, false));
    *response.status_mut() = plan.status;
    let headers = response.headers_mut();
    headers.insert("content-type", HeaderValue::from_static(plan.content_type));
    headers.insert(
        if plan.stream {
            "connect-accept-encoding"
        } else {
            "accept-encoding"
        },
        HeaderValue::from_static("gzip"),
    );
    if plan.gzip {
        headers.insert("connect-content-encoding", HeaderValue::from_static("gzip"));
    }
    // The original server flushes stream EOS, so never supplies a length. Its
    // unary writer only supplies an automatic length within the 2048-byte buffer.
    if !plan.stream && length <= 2048 {
        headers.insert(
            "content-length",
            HeaderValue::from_str(&length.to_string()).map_err(|_| Error::Unavailable)?,
        );
    }
    Ok(response)
}
