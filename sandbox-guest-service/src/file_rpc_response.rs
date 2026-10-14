//! Identity JSON unary replies, charged to their already admitted records.
//! HTTP request admission, protocol negotiation and RPC errors are separate.
use crate::owned_body::{self, OwnedBody, Owner, Payload};
use axum::{http::HeaderValue, response::Response};
use sandproof_guest_runtime::{file_protocol, protocol::ProtocolError};
use std::{fmt, sync::Arc};
use tokio::time::Instant;

#[cfg(test)]
mod tests;

/// The record budget is separate. Encoding retains at most this JSON payload
/// plus one bounded transmission chunk, under the original request capacity.
#[derive(Clone, Copy, Debug)]
pub struct Limits {
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
}
impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "file RPC response: {self:?}")
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

/// Preparation failed after the file operation. Keep its original capacity
/// while the caller decides how to deliver a failure; never replay the I/O.
pub struct Failure {
    error: Error,
    deadline: Instant,
    _owner: Owner,
}
impl Failure {
    #[must_use]
    pub const fn error(&self) -> Error {
        self.error
    }
    #[must_use]
    pub const fn deadline(&self) -> Instant {
        self.deadline
    }
}
impl fmt::Debug for Failure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("FileRpcResponseFailure")
            .field("error", &self.error)
            .finish_non_exhaustive()
    }
}

/// Encode bounded, completed file records and send under their original lease.
/// `records` must retain the original request/record capacity; `view` borrows
/// its wire representation without duplicating the entries. The caller has
/// already selected unary application/json with identity response encoding.
/// # Errors
/// Invalid limits, encoding overflow/failure, original expiry or lost worker.
/// These do not imply the completed file operation failed or can be retried.
pub async fn send<T: Send + Sync + 'static>(
    records: Arc<T>,
    view: for<'a> fn(&'a T) -> file_protocol::Response<'a>,
    limits: Limits,
    deadline: Instant,
) -> Result<Response, Failure> {
    let owner: Owner = records.clone();
    let result = async {
        if limits.wire_bytes == 0
            || limits.chunk_bytes == 0
            || limits.wire_bytes > isize::MAX as usize / 16
            || limits.chunk_bytes > isize::MAX as usize / 16
        {
            return Err(Error::InvalidConfiguration);
        }
        let (length, body) = OwnedBody::start(deadline, Arc::clone(&owner), move |_, _| {
            let bytes = file_protocol::encode_response(view(&records), limits.wire_bytes).map_err(
                |error| match error {
                    ProtocolError::ResourceExhausted => Error::ResponseLimit,
                    _ => Error::Encoding,
                },
            )?;
            Ok((bytes.len(), Payload::new(bytes, limits.chunk_bytes)))
        })
        .await?;
        let mut response = Response::new(body.into_body());
        response
            .headers_mut()
            .insert("content-type", HeaderValue::from_static("application/json"));
        // The original Connect handler advertises its default request decoder
        // even when this particular response is explicitly identity encoded.
        response
            .headers_mut()
            .insert("accept-encoding", HeaderValue::from_static("gzip"));
        if length <= 2048 {
            response.headers_mut().insert(
                "content-length",
                HeaderValue::from_str(&length.to_string()).map_err(|_| Error::Unavailable)?,
            );
        }
        Ok(response)
    }
    .await;
    result.map_err(|error| Failure {
        error,
        deadline,
        _owner: owner,
    })
}
