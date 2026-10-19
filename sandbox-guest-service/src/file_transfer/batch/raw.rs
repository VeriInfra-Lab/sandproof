use super::{BatchReply, FileStreamError, UploadBatch, UploadBodyError};
use crate::upload_body::{HttpBodyLimits, Input};
use axum::body::Body;

impl UploadBatch {
    /// A fresh raw request shares the batch's original admission/deadline.
    /// # Errors
    /// Missing query path, frame/length/EOF failures, budget or file I/O.
    pub async fn receive_raw(
        mut self,
        body: Body,
        expected: Option<u64>,
        limits: HttpBodyLimits,
    ) -> Result<BatchReply, UploadBodyError> {
        self.fresh()?;
        if self.query_path.is_none() {
            return Err(FileStreamError::InvalidArgument.into());
        }
        let mut input = Input::new(body, expected, limits, self.reservation.deadline)?;
        let mut file = self.start_file("", expected).await?;
        while let Some(chunk) = input.next().await? {
            for bytes in chunk.chunks(file.chunk_bytes()) {
                file.write(bytes).await?;
            }
        }
        file.finish().await?;
        self.finish().map_err(Into::into)
    }
}
