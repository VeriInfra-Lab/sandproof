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
    /// Formats the resolved filename after open. Automatic MIME uses the
    /// original query path and a caller-owned catalog; sniffing follows
    /// conditions inside the owned worker. Formal negotiation remains separate.
    /// # Errors
    /// Admission, stat/open, conditional/range plans or initial seek failures.
    /// A later seek/read/short read is an HTTP body error, not a second response.
    pub async fn download_identity<Z: LocalZone + Send + 'static>(
        &self,
        auth: AuthPermit,
        context: HttpFileContext,
        mut request: IdentityRequest,
        limits: Limits,
        local: Z,
    ) -> Result<IdentityDownload, IdentityDownloadError> {
        let (lease, deadline) = self.admit().map_err(IdentityDownloadError::Admission)?;
        let owner = Arc::new((auth, lease));
        let transfer = self.limits;
        if context.path.len() > transfer.file.path_bytes {
            return Err(IdentityDownloadError::Download(
                DownloadError::ResourceExhausted,
            ));
        }
        request.disposition_path = Some(context.path.as_bytes().to_vec());
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
                path_bytes: transfer.file.path_bytes,
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
