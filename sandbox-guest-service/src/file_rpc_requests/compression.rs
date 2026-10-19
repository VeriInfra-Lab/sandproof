use super::{Error, GzipError, Instant, watch};
pub use crate::request_gzip::CompressionStage;

pub(super) fn expand(
    payload: &[u8],
    limit: usize,
    chunk: usize,
    deadline: Instant,
    cancelled: &watch::Receiver<bool>,
) -> Result<Vec<u8>, Error> {
    crate::request_gzip::expand(payload, limit, chunk, deadline, cancelled)
        .map_err(|error| failure(error.stage, error.error))
}

fn failure(stage: CompressionStage, error: GzipError) -> Error {
    match error {
        GzipError::DeadlineExceeded => Error::DeadlineExceeded,
        GzipError::Unavailable => Error::Unavailable,
        GzipError::InputLimit
        | GzipError::DecodedLimit
        | GzipError::HeaderLimit
        | GzipError::MemberLimit => Error::BodyLimit,
        error => Error::Compression { stage, error },
    }
}
