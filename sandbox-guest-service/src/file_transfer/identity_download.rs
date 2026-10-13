use super::{FileStreamError, FileStreams, TransferIoError};
use crate::{
    AuthPermit,
    file_download::{DownloadError, IdentityDownload, IdentityLimits, IdentityRequest, Limits},
    files::HttpFileContext,
};
use sandproof_guest_runtime::{
    file_download::date::{LocalZone, Timestamp},
    filesystem::transfer::DownloadSource,
};
use std::{os::unix::fs::MetadataExt, sync::Arc};

#[cfg(test)]
mod tests;

impl FileStreams {
    /// Identity/range component under one original authorization and slot.
    /// MIME must already be known without pre-reading. Formal negotiation and
    /// MIME discovery remain caller obligations; no production route is added.
    /// # Errors
    /// Admission, stat/open, conditional/range plans or initial seek failures.
    /// A later seek/read/short read is an HTTP body error, not a second response.
    pub async fn download_identity<Z: LocalZone + Send + 'static>(
        &self,
        auth: AuthPermit,
        context: HttpFileContext,
        request: IdentityRequest,
        limits: Limits,
        local: Z,
    ) -> Result<IdentityDownload, IdentityDownloadError> {
        let (lease, deadline) = self.admit().map_err(IdentityDownloadError::Admission)?;
        let owner = Arc::new((auth, lease));
        let transfer = self.limits;
        IdentityDownload::open(
            move || {
                let (source, info) = DownloadSource::open_with_stat(&context.path, transfer)
                    .map_err(|error| match error {
                        TransferIoError::NotFound => DownloadError::SourceNotFound,
                        TransferIoError::InvalidArgument => DownloadError::SourceInvalid,
                        TransferIoError::ResourceExhausted => DownloadError::ResourceExhausted,
                        _ => DownloadError::Open,
                    })?;
                let nanos =
                    u32::try_from(info.mtime_nsec()).map_err(|_| DownloadError::SourceInvalid)?;
                let modified = Timestamp::new(info.mtime(), nanos)
                    .map_err(|_| DownloadError::SourceInvalid)?;
                Ok((source, modified))
            },
            request,
            IdentityLimits {
                plan: limits,
                chunk_bytes: transfer.chunk_bytes,
                file_bytes: transfer.total_bytes,
            },
            local,
            deadline,
            owner,
        )
        .await
        .map_err(IdentityDownloadError::Download)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IdentityDownloadError {
    Admission(FileStreamError),
    Download(DownloadError),
}
