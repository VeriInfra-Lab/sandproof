//! Bounded JSON unary replies, charged to their already admitted records.
//! Fixed account/header rejections share the same owned JSON response transport.
//! Request admission and protocol negotiation remain separate.
use crate::owned_body::{self, OwnedBody, Owner, Payload};
use axum::{
    http::{HeaderValue, StatusCode},
    response::Response,
};
use sandproof_guest_runtime::{file_protocol, protocol::ProtocolError};
use std::{fmt, sync::Arc};
use tokio::time::Instant;

mod rejection;
pub use rejection::{Rejection, rejected};
mod io;
pub use io::io_failure;
mod compression;
pub use compression::Encoding;

#[cfg(test)]
mod tests;

/// The record budget is separate. Identity retains this JSON payload plus one
/// transmission chunk. Gzip preparation additionally retains at most `wire_bytes`
/// compressed bytes and its fixed codec workspace, under original capacity.
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
    UnclassifiedFault,
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

/// Response preparation failed. Keep original request/result capacity while
/// the caller decides how to deliver a failure; never replay any completed I/O.
pub struct Failure {
    error: Error,
    deadline: Instant,
    _owner: Owner,
}
impl Failure {
    pub(crate) fn unclassified(deadline: Instant, owner: Owner) -> Self {
        Self {
            error: Error::UnclassifiedFault,
            deadline,
            _owner: owner,
        }
    }
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
    send_with_encoding(records, view, limits, deadline, Encoding::Identity).await
}

/// Encode successful records with the already selected unary response encoding.
/// Fixed Connect has no minimum gzip size here; errors use the separate identity
/// JSON sender. Raw JSON and compressed bytes each obey the configured cap.
/// # Errors
/// Invalid limits, encoding/compression limits, expiry or lost worker. A failed
/// response never authorizes replaying the completed filesystem operation.
pub async fn send_with_encoding<T: Send + Sync + 'static>(
    records: Arc<T>,
    view: for<'a> fn(&'a T) -> file_protocol::Response<'a>,
    limits: Limits,
    deadline: Instant,
    encoding: Encoding,
) -> Result<Response, Failure> {
    let owner: Owner = records.clone();
    send_encoded_with_encoding(
        owner,
        limits,
        deadline,
        StatusCode::OK,
        true,
        encoding,
        move || {
            file_protocol::encode_response(view(&records), limits.wire_bytes).map_err(|error| {
                match error {
                    ProtocolError::ResourceExhausted => Error::ResponseLimit,
                    _ => Error::Encoding,
                }
            })
        },
    )
    .await
}

async fn send_encoded(
    owner: Owner,
    limits: Limits,
    deadline: Instant,
    status: StatusCode,
    accepts_gzip: bool,
    encode: impl FnOnce() -> Result<Vec<u8>, Error> + Send + 'static,
) -> Result<Response, Failure> {
    send_encoded_with_encoding(
        owner,
        limits,
        deadline,
        status,
        accepts_gzip,
        Encoding::Identity,
        encode,
    )
    .await
}

async fn send_encoded_with_encoding(
    owner: Owner,
    limits: Limits,
    deadline: Instant,
    status: StatusCode,
    accepts_gzip: bool,
    encoding: Encoding,
    encode: impl FnOnce() -> Result<Vec<u8>, Error> + Send + 'static,
) -> Result<Response, Failure> {
    let result = async {
        if limits.wire_bytes == 0
            || limits.chunk_bytes == 0
            || limits.wire_bytes > isize::MAX as usize / 16
            || limits.chunk_bytes > isize::MAX as usize / 16
        {
            return Err(Error::InvalidConfiguration);
        }
        let (length, body) = OwnedBody::start(deadline, Arc::clone(&owner), move |_, cancelled| {
            let bytes = encode()?;
            let bytes = match encoding {
                Encoding::Identity => bytes,
                Encoding::Gzip => compression::compress(&bytes, limits, deadline, &cancelled)?,
            };
            Ok((bytes.len(), Payload::new(bytes, limits.chunk_bytes)))
        })
        .await?;
        let mut response = Response::new(body.into_body());
        *response.status_mut() = status;
        response
            .headers_mut()
            .insert("content-type", HeaderValue::from_static("application/json"));
        if encoding == Encoding::Gzip {
            response
                .headers_mut()
                .insert("content-encoding", HeaderValue::from_static("gzip"));
        }
        // The Connect handler advertises its request decoder. authn's outer
        // ErrorWriter does not pass through that handler and adds no such header.
        if accepts_gzip {
            response
                .headers_mut()
                .insert("accept-encoding", HeaderValue::from_static("gzip"));
        }
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
