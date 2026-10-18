use super::{Error, Request};
use crate::{request_gzip, upload_body::GzipError};
use sandproof_guest_runtime::protocol;
use tokio::{sync::watch, time::Instant};

#[cfg(test)]
mod tests;

#[cfg(test)]
pub(super) fn decode(
    method: &str,
    payload: &[u8],
    limit: usize,
    chunk: usize,
    deadline: Instant,
    cancelled: &watch::Receiver<bool>,
) -> Result<Request, Error> {
    decode_selected(method, payload, limit, chunk, deadline, cancelled, true)
}

#[cfg(test)]
pub(super) fn decode_selected(
    method: &str,
    payload: &[u8],
    limit: usize,
    chunk: usize,
    deadline: Instant,
    cancelled: &watch::Receiver<bool>,
    gzip: bool,
) -> Result<Request, Error> {
    decode_limited(
        method,
        payload,
        limit,
        chunk,
        deadline,
        cancelled,
        (gzip, None),
    )
}

pub(super) fn decode_limited(
    method: &str,
    payload: &[u8],
    limit: usize,
    chunk: usize,
    deadline: Instant,
    cancelled: &watch::Receiver<bool>,
    selection: (bool, Option<usize>),
) -> Result<Request, Error> {
    protocol::decode_request_observed_limited(
        method,
        payload,
        limit,
        selection.0,
        selection.1,
        |bytes| {
            request_gzip::expand(bytes, limit, chunk, deadline, cancelled).map_err(|failure| {
                match failure.error {
                    GzipError::DeadlineExceeded => Error::DeadlineExceeded,
                    GzipError::Unavailable => Error::Unavailable,
                    GzipError::InputLimit
                    | GzipError::DecodedLimit
                    | GzipError::HeaderLimit
                    | GzipError::MemberLimit => Error::BodyLimit,
                    error => Error::Compression {
                        stage: failure.stage,
                        error,
                    },
                }
            })
        },
    )
}
