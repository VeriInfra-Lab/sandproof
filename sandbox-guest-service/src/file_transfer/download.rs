use super::{FileStreams, IdentityDownloadError, TransferIoError};
use crate::{
    AuthPermit,
    file_download::{
        Download, DownloadError, IdentityLimits, IdentityRequest, Limits, NegotiationLimits,
    },
    files::HttpFileContext,
};
use sandproof_guest_runtime::{
    file_download::date::{LocalZone, Timestamp},
    filesystem::transfer::{DownloadPath, DownloadPathError},
};
use std::{os::unix::fs::MetadataExt, sync::Arc};

#[cfg(test)]
mod tests;

impl FileStreams {
    /// A single admitted stat -> selection -> open -> streaming request. The
    /// source, gzip/identity worker and response bytes retain original auth.
    /// # Errors
    /// Admission, stat/directory, encoding (with Vary stage), open or preparation.
    /// Fixed post-resolution filesystem/negotiation errors become owned responses.
    /// Authentication/signing and context-resolution errors are not implemented here.
    pub async fn download_negotiated<Z: LocalZone + Send + 'static>(
        &self,
        auth: AuthPermit,
        context: HttpFileContext,
        mut request: IdentityRequest,
        limits: DownloadLimits,
        local: Z,
    ) -> Result<Download, IdentityDownloadError> {
        let (lease, deadline) = self.admit().map_err(IdentityDownloadError::Admission)?;
        let owner = Arc::new((auth, lease));
        let transfer = self.limits;
        if context.path.len() > transfer.file.path_bytes {
            return Err(IdentityDownloadError::Download(
                DownloadError::ResourceExhausted,
            ));
        }
        request.disposition_path = Some(context.path.as_bytes().to_vec());
        Download::open_negotiated(
            move || {
                let path =
                    DownloadPath::inspect_detailed(context.path, transfer).map_err(
                        |e| match e {
                            DownloadPathError::Io(Some(errno)) => DownloadError::StatIo(errno),
                            DownloadPathError::Directory => DownloadError::Directory,
                            DownloadPathError::Rejected(TransferIoError::InvalidArgument) => {
                                DownloadError::SourceInvalid
                            }
                            DownloadPathError::Rejected(TransferIoError::ResourceExhausted) => {
                                DownloadError::ResourceExhausted
                            }
                            _ => DownloadError::Stat,
                        },
                    )?;
                let metadata = path.metadata();
                let nanos = u32::try_from(metadata.mtime_nsec())
                    .map_err(|_| DownloadError::SourceInvalid)?;
                let modified = Timestamp::new(metadata.mtime(), nanos)
                    .map_err(|_| DownloadError::SourceInvalid)?;
                Ok((modified, move || {
                    path.open_detailed()
                        .map(|(source, _)| source)
                        .map_err(|e| match e {
                            DownloadPathError::Io(Some(errno)) => DownloadError::OpenIo(errno),
                            _ => DownloadError::Open,
                        })
                }))
            },
            request,
            NegotiationLimits {
                identity: IdentityLimits {
                    plan: limits.response,
                    chunk_bytes: transfer.chunk_bytes,
                    file_bytes: transfer.total_bytes,
                    path_bytes: transfer.file.path_bytes,
                },
                encoding: limits.encoding,
                compressed_bytes: limits.compressed_bytes,
            },
            local,
            deadline,
            owner,
        )
        .await
        .map_err(IdentityDownloadError::Download)
    }
}
#[derive(Clone, Copy)]
pub struct DownloadLimits {
    pub response: Limits,
    pub encoding: crate::file_encoding::Limits,
    pub compressed_bytes: u64,
}
