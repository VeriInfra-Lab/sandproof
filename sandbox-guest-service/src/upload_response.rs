//! Fixed upload envelopes, under the original retained upload capacity.
//! It reports observed file outcomes; delivery failure never rolls them back.
use crate::{
    go_json,
    owned_body::{self, OwnedBody, Owner, Source},
};
use axum::{http::HeaderValue, response::Response};
use sandproof_guest_runtime::{file_mime::basename, file_upload::UploadResult};
use std::fmt;
use tokio::time::Instant;

#[cfg(test)]
mod tests;

mod errors;
pub use errors::io_failure;
mod multipart_read;
mod rejection;
pub use rejection::{Rejection, rejected};
mod gzip_header;
pub use gzip_header::GzipHeaderError;
mod gzip_read;
pub use gzip_read::GzipReadError;

/// Result entries have their existing path/metadata budget. This separately
/// bounds the escaped JSON payload and each retained transmission chunk.
/// The worker may retain the full payload plus one chunk, not just one chunk.
#[derive(Clone, Copy, Debug)]
pub struct Limits {
    pub wire_bytes: usize,
    pub chunk_bytes: usize,
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
        write!(f, "upload response: {self:?}")
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

/// Send already retained, completed entries under the exact original guard.
/// Caller bounds the source entries and supplies their original deadline;
/// this component does not perform admission or obtain file authorization.
/// # Errors
/// Invalid response budgets, escaped payload limit, original expiry or lost
/// worker. No result is a request to replay or roll back the completed writes.
pub async fn success(
    results: Vec<UploadResult>,
    limits: Limits,
    deadline: Instant,
    owner: Owner,
) -> Result<Response, Error> {
    send(limits, deadline, owner, move || {
        encode(&results, limits.wire_bytes)
    })
    .await
}

pub(crate) async fn send(
    limits: Limits,
    deadline: Instant,
    owner: Owner,
    encode: impl FnOnce() -> Result<Vec<u8>, Error> + Send + 'static,
) -> Result<Response, Error> {
    if limits.wire_bytes == 0
        || limits.chunk_bytes == 0
        || limits.wire_bytes > isize::MAX as usize / 16
        || limits.chunk_bytes > isize::MAX as usize / 16
    {
        return Err(Error::InvalidConfiguration);
    }
    let (length, body) = OwnedBody::start(deadline, owner, move |_, _| {
        let bytes = encode()?;
        Ok((
            bytes.len(),
            Payload {
                bytes,
                offset: 0,
                chunk_bytes: limits.chunk_bytes,
            },
        ))
    })
    .await?;
    let mut response = Response::new(body.into_body());
    // The pinned handler writes JSON with no explicit Content-Type. Go's HTTP
    // writer sniffs this array as text/plain. Its 2048-byte buffer adds a length
    // only when the complete payload fits; larger HTTP/1.1 replies are chunked.
    response.headers_mut().insert(
        "content-type",
        HeaderValue::from_static("text/plain; charset=utf-8"),
    );
    if length <= 2048 {
        response.headers_mut().insert(
            "content-length",
            HeaderValue::from_str(&length.to_string()).map_err(|_| Error::Unavailable)?,
        );
    }
    Ok(response)
}

fn encode(results: &[UploadResult], limit: usize) -> Result<Vec<u8>, Error> {
    go_json::bounded(limit as u64, |put| {
        put(b"[")?;
        for (index, entry) in results.iter().enumerate() {
            if index != 0 {
                put(b",")?;
            }
            put(b"{")?;
            if !entry.metadata.is_empty() {
                put(b"\"metadata\":{")?;
                for (index, (key, value)) in entry.metadata.iter().enumerate() {
                    if index != 0 {
                        put(b",")?;
                    }
                    put(b"\"")?;
                    go_json::string_content(key.as_bytes(), put)?;
                    put(b"\":\"")?;
                    go_json::string_content(value.as_bytes(), put)?;
                    put(b"\"")?;
                }
                put(b"},")?;
            }
            put(b"\"name\":\"")?;
            go_json::string_content(basename(entry.path.as_bytes()), put)?;
            put(b"\",\"path\":\"")?;
            go_json::string_content(entry.path.as_bytes(), put)?;
            put(b"\",\"type\":\"file\"}")?;
        }
        put(b"]")
    })
    .map_err(|_| Error::ResponseLimit)
}
struct Payload {
    bytes: Vec<u8>,
    offset: usize,
    chunk_bytes: usize,
}
impl Source<Error> for Payload {
    fn next_chunk(&mut self) -> Result<Option<Vec<u8>>, Error> {
        if self.offset == self.bytes.len() {
            return Ok(None);
        }
        if self.offset == 0 && self.bytes.len() <= self.chunk_bytes {
            return Ok(Some(std::mem::take(&mut self.bytes)));
        }
        let count = (self.bytes.len() - self.offset).min(self.chunk_bytes);
        let end = self.offset + count;
        let chunk = self.bytes[self.offset..end].to_vec();
        self.offset = end;
        Ok(Some(chunk))
    }
}
