//! Original authn middleware replies for a bounded, observed missing explicit
//! Basic account. This precedes Connect method/codec/timeout processing.
use super::super::Error as RequestError;
use super::reply;
#[cfg(any(target_os = "linux", test))]
use crate::{finite_body, go_json};
use crate::{
    owned_body::{self, Owner},
    upload_response::UnknownUser,
};
use axum::http::HeaderValue;
#[cfg(any(target_os = "linux", test))]
use axum::{
    body::Bytes,
    http::{HeaderMap, Method, StatusCode},
    response::Response,
};
use std::{fmt, sync::Arc};
use tokio::time::Instant;

#[cfg(target_os = "linux")]
mod owned;
#[cfg(target_os = "linux")]
pub use owned::Failure;
#[cfg(test)]
mod tests;

#[derive(Clone, Copy, Debug)]
pub struct Limits {
    pub header_bytes: usize,
    pub body: reply::Limits,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    InvalidConfiguration,
    ResponseLimit,
    DeadlineExceeded,
    Unavailable,
    UnclassifiedFault,
}
impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "command account response: {self:?}")
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

struct Snapshot {
    post: bool,
    head: bool,
    content_type: Option<HeaderValue>,
    version: Option<HeaderValue>,
    input_bytes: usize,
}
#[cfg(any(target_os = "linux", test))]
impl Snapshot {
    // Only called after the original complete header budget has been enforced.
    fn capture(headers: &HeaderMap, method: &Method, input_bytes: usize) -> Self {
        Self {
            post: method == Method::POST,
            head: method == Method::HEAD,
            content_type: headers.get("content-type").cloned(),
            version: headers.get("connect-protocol-version").cloned(),
            input_bytes,
        }
    }
    fn stream_type(&self, limit: usize) -> Result<Option<String>, Error> {
        if !self.post {
            return Ok(None);
        }
        let raw = self
            .content_type
            .as_ref()
            .map_or(b"".as_slice(), HeaderValue::as_bytes);
        let raw = std::str::from_utf8(raw).map_err(|_| Error::UnclassifiedFault)?;
        let canonical =
            sandproof_guest_runtime::protocol::canonical_content_type(raw, self.input_bytes, limit)
                .map_err(|error| match error {
                    sandproof_guest_runtime::protocol::ProtocolError::ResourceExhausted => {
                        Error::ResponseLimit
                    }
                    _ => Error::UnclassifiedFault,
                })?;
        if canonical == "application/grpc"
            || canonical.starts_with("application/grpc+")
            || canonical == "application/grpc-web"
            || canonical.starts_with("application/grpc-web+")
        {
            return Err(Error::UnclassifiedFault);
        }
        let version = self
            .version
            .as_ref()
            .map_or(b"".as_slice(), HeaderValue::as_bytes);
        Ok(
            (canonical.starts_with("application/connect+") && matches!(version, b"" | b"1"))
                .then(|| canonical.into_owned()),
        )
    }
}

struct Source {
    user: Option<UnknownUser>,
    request_error: RequestError,
    snapshot: Snapshot,
    deadline: Instant,
    _owner: Owner,
}
pub struct ReplyFailure {
    error: Error,
    source: Arc<Source>,
}
impl ReplyFailure {
    #[must_use]
    pub const fn error(&self) -> Error {
        self.error
    }
    #[must_use]
    pub fn deadline(&self) -> Instant {
        self.source.deadline
    }
    #[must_use]
    pub fn request_error(&self) -> RequestError {
        self.source.request_error
    }
}
impl fmt::Debug for ReplyFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CommandAccountReplyFailure")
            .field("error", &self.error)
            .finish_non_exhaustive()
    }
}

#[cfg(any(target_os = "linux", test))]
async fn send(source: Source, limits: Limits) -> Result<Response, ReplyFailure> {
    let source = Arc::new(source);
    send_inner(source.clone(), limits)
        .await
        .map_err(|error| ReplyFailure { error, source })
}
#[cfg(any(target_os = "linux", test))]
async fn send_inner(source: Arc<Source>, limits: Limits) -> Result<Response, Error> {
    if limits.header_bytes == 0
        || limits.header_bytes > isize::MAX as usize / 16
        || limits.body.wire_bytes == 0
        || limits.body.wire_bytes > u32::MAX as usize
        || limits.body.wire_bytes > isize::MAX as usize / 16
        || limits.body.chunk_bytes == 0
        || limits.body.chunk_bytes > isize::MAX as usize / 16
    {
        return Err(Error::InvalidConfiguration);
    }
    if source.user.is_none() {
        return Err(Error::UnclassifiedFault);
    }
    let content_type = source.snapshot.stream_type(limits.header_bytes)?;
    let stream = content_type.is_some();
    let head = source.snapshot.head;
    let deadline = source.deadline;
    let worker_source = source.clone();
    let payload = finite_body::prepare(deadline, source.clone(), move |_| {
        encode(
            worker_source
                .user
                .as_ref()
                .ok_or(Error::UnclassifiedFault)?,
            stream,
            limits.body.wire_bytes,
        )
    })
    .await?;
    let length = payload.length();
    let mut response = Response::new(payload.into_body(limits.body.chunk_bytes, deadline, head));
    *response.status_mut() = if stream {
        StatusCode::OK
    } else {
        StatusCode::UNAUTHORIZED
    };
    let content_type = if let Some(content_type) = content_type {
        // Dynamic response-header clones also retain original request capacity.
        HeaderValue::from_maybe_shared(Bytes::from_owner(Header {
            bytes: content_type.into_bytes(),
            _owner: source,
        }))
        .map_err(|_| Error::UnclassifiedFault)?
    } else {
        HeaderValue::from_static("application/json")
    };
    response.headers_mut().insert("content-type", content_type);
    // Outer ErrorWriter doesn't flush EOS and never negotiates compression or
    // advertises encodings. HEAD suppresses bytes but preserves the known length.
    if length <= 2048 {
        response.headers_mut().insert(
            "content-length",
            HeaderValue::from_str(&length.to_string()).map_err(|_| Error::Unavailable)?,
        );
    }
    Ok(response)
}
#[cfg(any(target_os = "linux", test))]
struct Header {
    bytes: Vec<u8>,
    _owner: Owner,
}
#[cfg(any(target_os = "linux", test))]
impl AsRef<[u8]> for Header {
    fn as_ref(&self) -> &[u8] {
        &self.bytes
    }
}

#[cfg(any(target_os = "linux", test))]
fn encode(user: &UnknownUser, stream: bool, limit: usize) -> Result<Vec<u8>, Error> {
    let bytes = go_json::bounded(limit as u64, |put| {
        if stream {
            put(&[2, 0, 0, 0, 0])?;
            put(br#"{"error":"#)?;
        }
        put(br#"{"code":"unauthenticated","message":"invalid username: '"#)?;
        go_json::string_content(user.name().as_bytes(), put)?;
        put(if stream { br#"'"}}"# } else { br#"'"}"# })
    })
    .map_err(|_| Error::ResponseLimit)?;
    if !stream {
        return Ok(bytes);
    }
    let mut bytes = bytes;
    let length = u32::try_from(bytes.len() - 5).map_err(|_| Error::ResponseLimit)?;
    bytes[1..5].copy_from_slice(&length.to_be_bytes());
    Ok(bytes)
}
