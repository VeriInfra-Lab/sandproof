//! Execute the original descriptor under one blocking worker and caller guard.
use super::{BodyPlan, Error, Limits, Preparation, ResponsePlan, Segment, prepare};
pub use crate::owned_body::OwnedChunk as DownloadChunk;
use crate::owned_body::{self, OwnedBody, Owner, Source};
use axum::{
    body::{Body, Bytes},
    http::{HeaderMap, HeaderValue, Method, StatusCode},
    response::Response,
};
use futures_util::stream;
use sandproof_guest_runtime::file_download::date::{LocalZone, Timestamp};
use std::{
    fmt,
    io::{self, Read, Seek, SeekFrom},
    sync::Arc,
};
use tokio::{sync::watch, time::Instant};

#[cfg(test)]
mod tests;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DownloadError {
    Plan(Error),
    InvalidConfiguration,
    SourceNotFound,
    SourceInvalid,
    ResourceExhausted,
    Open,
    LengthSeek,
    RangeSeek,
    Read,
    Incomplete,
    DeadlineExceeded,
    Unavailable,
}
impl fmt::Display for DownloadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "identity download: {self:?}")
    }
}
impl std::error::Error for DownloadError {}
impl From<Error> for DownloadError {
    fn from(error: Error) -> Self {
        Self::Plan(error)
    }
}
impl owned_body::Failure for DownloadError {
    fn deadline() -> Self {
        Self::DeadlineExceeded
    }
    fn unavailable() -> Self {
        Self::Unavailable
    }
}

/// Known MIME type only: extension lookup/sniffing belongs to the caller's
/// future MIME adapter. It must not pre-read a file to manufacture this value.
/// The trusted boundary must be generated entropy, never request/path input.
pub struct IdentityRequest {
    pub method: Method,
    pub headers: HeaderMap,
    pub response: HeaderMap,
    pub content_type: HeaderValue,
    pub boundary: Option<String>,
}
#[derive(Clone, Copy)]
pub struct IdentityLimits {
    pub plan: Limits,
    pub chunk_bytes: usize,
    /// Selected file bytes, separate from wire bytes including MIME overhead.
    pub file_bytes: u64,
}

/// Ready headers follow successful preparation and the initial single-range
/// seek. The retained handle and all chunks keep the original guard alive.
pub struct IdentityDownload {
    status: StatusCode,
    headers: HeaderMap,
    body: OwnedBody<DownloadError>,
    _owner: Owner,
}
impl IdentityDownload {
    /// Open and prepare on the same owned worker used for every seek/read.
    /// `open` returns the original descriptor and pre-open stat timestamp.
    /// Caller already owns admission/auth and the known MIME type; this is not
    /// encoding negotiation or a production file route. No body read-ahead.
    /// # Errors
    /// Open/plan/length/single-range seek failure, budgets or original deadline.
    /// Later read/seek/short-read failures stay body errors, never HTTP EOF.
    pub async fn open<R, F, Z>(
        open: F,
        request: IdentityRequest,
        limits: IdentityLimits,
        local: Z,
        deadline: Instant,
        owner: Owner,
    ) -> Result<Self, DownloadError>
    where
        R: Read + Seek + 'static,
        F: FnOnce() -> Result<(R, Timestamp), DownloadError> + Send + 'static,
        Z: LocalZone + Send + 'static,
    {
        if limits.chunk_bytes == 0 || limits.chunk_bytes > isize::MAX as usize / 16 {
            return Err(DownloadError::InvalidConfiguration);
        }
        let ((status, headers), body) =
            OwnedBody::start(deadline, Arc::clone(&owner), move |_, cancel| {
                let (mut source, modified) = open()?;
                let control = Control { deadline, cancel };
                control.check()?;
                let head = request.method == Method::HEAD;
                let plan = match prepare(
                    &request.method,
                    &request.headers,
                    request.response,
                    modified,
                    limits.plan,
                    &local,
                )? {
                    Preparation::Finished(plan) => plan,
                    Preparation::Continue(next) => {
                        let size = control.seek(
                            &mut source,
                            SeekFrom::End(0),
                            DownloadError::LengthSeek,
                        )?;
                        control.seek(&mut source, SeekFrom::Start(0), DownloadError::LengthSeek)?;
                        let plan =
                            next.finish(size, request.content_type, request.boundary.as_deref())?;
                        if plan.file_bytes > limits.file_bytes {
                            return Err(DownloadError::ResourceExhausted);
                        }
                        if let Some(start) = plan.required_seek {
                            control.seek(
                                &mut source,
                                SeekFrom::Start(start),
                                DownloadError::RangeSeek,
                            )?;
                        }
                        plan
                    }
                };
                let ResponsePlan {
                    status,
                    headers,
                    mut body,
                    ..
                } = plan;
                if head {
                    body = BodyPlan::Empty;
                }
                let source = if matches!(body, BodyPlan::Empty | BodyPlan::Error(_)) {
                    drop(source);
                    None
                } else {
                    Some(source)
                };
                control.check()?;
                let multi = matches!(body, BodyPlan::Multipart { .. });
                Ok((
                    (status, headers),
                    Reader {
                        source,
                        body,
                        index: 0,
                        current: None,
                        control,
                        multi,
                        chunk_bytes: limits.chunk_bytes,
                    },
                ))
            })
            .await?;
        Ok(Self {
            status,
            headers,
            body,
            _owner: owner,
        })
    }
    #[must_use]
    pub const fn status(&self) -> StatusCode {
        self.status
    }
    #[must_use]
    pub const fn headers(&self) -> &HeaderMap {
        &self.headers
    }
    /// Release all prior byte clones before requesting more data.
    /// # Errors
    /// Original deadline, source failure, short read or poisoned wait.
    pub async fn next(&mut self) -> Result<Option<DownloadChunk>, DownloadError> {
        self.body.next().await
    }
    #[must_use]
    pub fn into_response(mut self) -> Response {
        let status = self.status;
        let headers = std::mem::take(&mut self.headers);
        let body = Body::from_stream(stream::unfold(Some(self), |state| async move {
            let mut stream = state?;
            match stream.next().await {
                Ok(Some(chunk)) => Some((Ok(Bytes::from_owner(chunk)), Some(stream))),
                Ok(None) => None,
                Err(error) => Some((Err(error), None)),
            }
        }));
        let mut response = Response::new(body);
        *response.status_mut() = status;
        *response.headers_mut() = headers;
        response
    }
}

struct Control {
    deadline: Instant,
    cancel: watch::Receiver<bool>,
}
impl Control {
    fn check(&self) -> Result<(), DownloadError> {
        owned_body::check_active(self.deadline, &self.cancel)
    }
    fn seek(
        &self,
        source: &mut impl Seek,
        offset: SeekFrom,
        failure: DownloadError,
    ) -> Result<u64, DownloadError> {
        self.check()?;
        let result = source.seek(offset);
        self.check()?;
        result.map_err(|_| failure)
    }
}
enum Current {
    Bytes(Vec<u8>, usize),
    File(u64),
}
struct Reader<R> {
    source: Option<R>,
    body: BodyPlan,
    index: usize,
    current: Option<Current>,
    control: Control,
    multi: bool,
    chunk_bytes: usize,
}
impl<R: Read + Seek> Source<DownloadError> for Reader<R> {
    fn next_chunk(&mut self) -> Result<Option<Vec<u8>>, DownloadError> {
        loop {
            self.control.check()?;
            match &mut self.current {
                Some(Current::Bytes(bytes, offset)) if *offset < bytes.len() => {
                    let end = bytes.len().min(*offset + self.chunk_bytes);
                    let output = bytes[*offset..end].to_vec();
                    *offset = end;
                    return Ok(Some(output));
                }
                Some(Current::File(remaining)) if *remaining > 0 => {
                    let size = usize::try_from((*remaining).min(self.chunk_bytes as u64))
                        .map_err(|_| DownloadError::ResourceExhausted)?;
                    let mut bytes = vec![0; size];
                    let result = self
                        .source
                        .as_mut()
                        .ok_or(DownloadError::Unavailable)?
                        .read(&mut bytes);
                    self.control.check()?;
                    match result {
                        Ok(0) => return Err(DownloadError::Incomplete),
                        Ok(n) if n <= size => {
                            *remaining -= n as u64;
                            bytes.truncate(n);
                            return Ok(Some(bytes));
                        }
                        Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                        _ => return Err(DownloadError::Read),
                    }
                }
                _ => self.current = None,
            }
            let Some(segment) = self.body.segment(self.index) else {
                return Ok(None);
            };
            self.index += 1;
            self.current = Some(match segment {
                Segment::Bytes(bytes) => Current::Bytes(bytes, 0),
                Segment::File(range) => {
                    if self.multi {
                        self.control.seek(
                            self.source.as_mut().ok_or(DownloadError::Unavailable)?,
                            SeekFrom::Start(range.start),
                            DownloadError::RangeSeek,
                        )?;
                    }
                    Current::File(range.length)
                }
            });
        }
    }
}
