use super::{BatchReply, FileStreamError, MultipartLimits, UploadBatch, UploadBodyError};
use crate::upload_body::{GzipBody, GzipLimits};
use axum::body::Body;
use std::sync::Arc;

impl UploadBatch {
    pub(super) fn fresh(&self) -> Result<(), UploadBodyError> {
        if !self.ready || !self.results.is_empty() || self.received != 0 {
            Err(FileStreamError::InvalidArgument.into())
        } else {
            Ok(())
        }
    }

    async fn gzip_body(
        &self,
        body: Body,
        expected: Option<u64>,
        limits: GzipLimits,
    ) -> Result<GzipBody, UploadBodyError> {
        self.fresh()?;
        // One original request: no second admission or restarted deadline.
        // Worker and any retained decoded chunk keep this exact authorization.
        let owner = Arc::new((Arc::clone(&self.auth), self.reservation.clone()));
        GzipBody::open(body, expected, limits, self.reservation.deadline, owner)
            .await
            .map_err(UploadBodyError::GzipPreflight)
    }

    /// Consume a fresh batch as one gzip-encoded raw upload. The query path must
    /// be present, including an explicitly empty path. `expected` is compressed
    /// HTTP length, never the uncompressed file length. Header validation occurs
    /// before opening/truncating the file; metadata follows full gzip/HTTP EOF.
    ///
    /// # Errors
    /// Header/decode/HTTP failures, missing path, original expiry, quotas or file
    /// I/O. Failed streams retain already written bytes without metadata finish.
    pub async fn receive_gzip_raw(
        self,
        body: Body,
        expected: Option<u64>,
        limits: GzipLimits,
    ) -> Result<BatchReply, UploadBodyError> {
        let input = self.gzip_body(body, expected, limits).await?;
        self.receive_gzip_input(input).await
    }

    pub(in crate::file_transfer) async fn receive_gzip_input(
        mut self,
        mut input: GzipBody,
    ) -> Result<BatchReply, UploadBodyError> {
        self.fresh()?;
        if self.query_path.is_none() {
            return Err(FileStreamError::InvalidArgument.into());
        }
        let mut file = self.start_file("", None).await?;
        while let Some(chunk) = input.next().await? {
            for bytes in chunk.as_ref().chunks(file.chunk_bytes()) {
                file.write(bytes).await?;
            }
        }
        file.finish().await?;
        self.finish().map_err(Into::into)
    }

    /// Gzip HTTP -> MIME -> the original sequential file batch. Compressed HTTP,
    /// decoded MIME and file/result budgets are all explicit and independent.
    /// Only decoded gzip plus actual HTTP EOF can complete the MIME input.
    ///
    /// # Errors
    /// Any gzip/HTTP/MIME/file failure, original expiry or budget violation.
    /// Earlier completed files and the current written prefix are not rolled back.
    pub async fn receive_gzip_multipart(
        self,
        content_type: &str,
        body: Body,
        expected: Option<u64>,
        gzip: GzipLimits,
        multipart: MultipartLimits,
    ) -> Result<BatchReply, UploadBodyError> {
        let input = self.gzip_body(body, expected, gzip).await?;
        self.receive_multipart_source(content_type, input.into_body(), None, multipart, true)
            .await
    }
}
