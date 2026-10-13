use super::{FileStreamError, FileStreams, Lease, TransferIoError};
use crate::{
    AuthPermit,
    files::HttpFileContext,
    upload_body::{GzipBody, GzipChunk, GzipError},
};
use axum::body::{Body, Bytes};
use futures_util::stream;
use sandproof_guest_runtime::{filesystem::transfer::DownloadSource, gzip::EncodeLimits};
use std::sync::Arc;

#[cfg(test)]
mod tests;

impl FileStreams {
    /// Encode the original open descriptor to actual EOF under one admission
    /// and the original absolute deadline. No length is advertised or sought.
    /// The caller supplies a separate compressed-byte budget; the pool's raw
    /// byte budget still applies. Headers/encoding selection belong to caller.
    /// # Errors
    /// Admission/path/open failure or original deadline. Midstream failures
    /// remain body errors; they never become complete gzip or HTTP EOF.
    pub async fn download_gzip(
        &self,
        auth: AuthPermit,
        context: HttpFileContext,
        encoded_bytes: u64,
    ) -> Result<GzipDownload, FileStreamError> {
        let (lease, deadline) = self.admit()?;
        let owner = Arc::new((auth, Arc::clone(&lease)));
        let limits = self.limits;
        let body = GzipBody::encode(
            move || {
                DownloadSource::open(&context.path, limits).map_err(|error| match error {
                    TransferIoError::NotFound => GzipError::SourceNotFound,
                    TransferIoError::InvalidArgument => GzipError::SourceInvalid,
                    TransferIoError::ResourceExhausted => GzipError::InputLimit,
                    _ => GzipError::Transport,
                })
            },
            EncodeLimits {
                chunk_bytes: limits.chunk_bytes,
                decoded_bytes: limits.total_bytes,
                encoded_bytes,
            },
            deadline,
            owner,
        )
        .await
        .map_err(map_error)?;
        Ok(GzipDownload {
            body,
            _lease: lease,
        })
    }
}

/// Retains admission until the handle, worker and all returned byte owners are
/// released. Returned chunks also retain original authorization. No snapshot
/// or Content-Length is promised for this compressed stream.
pub struct GzipDownload {
    body: GzipBody,
    _lease: Arc<Lease>,
}
impl GzipDownload {
    /// Release every prior chunk/Bytes clone before awaiting the next chunk.
    /// # Errors
    /// File read, raw/compressed budget, cancellation or original deadline.
    pub async fn next(&mut self) -> Result<Option<GzipChunk>, FileStreamError> {
        self.body.next().await.map_err(map_error)
    }

    /// Preserves producer/transport ownership; only confirmed completion is EOF.
    pub fn into_body(self) -> Body {
        Body::from_stream(stream::unfold(Some(self), |state| async move {
            let mut stream = state?;
            match stream.body.next().await {
                Ok(Some(chunk)) => Some((Ok(Bytes::from_owner(chunk)), Some(stream))),
                Ok(None) => None,
                Err(error) => Some((Err(error), None)),
            }
        }))
    }
}

fn map_error(error: GzipError) -> FileStreamError {
    match error {
        GzipError::SourceNotFound => FileStreamError::Operation(TransferIoError::NotFound),
        GzipError::SourceInvalid => FileStreamError::Operation(TransferIoError::InvalidArgument),
        GzipError::InvalidConfiguration => FileStreamError::InvalidArgument,
        GzipError::InputLimit | GzipError::DecodedLimit => FileStreamError::ResourceExhausted,
        GzipError::DeadlineExceeded => FileStreamError::DeadlineExceeded,
        GzipError::Unavailable => FileStreamError::Unavailable,
        _ => FileStreamError::Operation(TransferIoError::Internal),
    }
}
