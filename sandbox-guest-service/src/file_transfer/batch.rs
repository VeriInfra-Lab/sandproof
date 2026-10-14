//! Sequential upload composition. The caller parses form boundaries and invokes
//! finish only after valid part/request endings. No rollback or HTTP parser here.
use super::{AuthPermit, FileStreamError, FileStreams, Reservation, UploadStream, timely};
use crate::files::HttpFileSelection;
use sandproof_guest_runtime::filesystem::transfer::{UploadResult, validate_metadata};
use std::{collections::BTreeMap, sync::Arc};

mod gzip;
mod multipart;
mod raw;
#[cfg(test)]
mod tests;
pub use multipart::{MultipartError, MultipartLimits, UploadBodyError};

/// Explicit per-request limits, in addition to the pool's per-file limits.
/// `total_bytes` counts file payloads; framing/ignored fields need their own
/// transport budget. Retained bytes count path and metadata string payloads.
#[derive(Clone, Copy, Debug)]
pub struct BatchLimits {
    pub files: usize,
    pub total_bytes: u64,
    pub retained_bytes: usize,
}

impl BatchLimits {
    pub(super) fn validate(self) -> Result<Self, FileStreamError> {
        if self.files == 0
            || self.files > isize::MAX as usize / 256
            || self.retained_bytes == 0
            || self.retained_bytes > isize::MAX as usize / 16
            || self.total_bytes == 0
        {
            return Err(FileStreamError::InvalidArgument);
        }
        Ok(self)
    }
}

impl FileStreams {
    /// One authorization, account snapshot, absolute deadline and pool slot for
    /// all sequential file parts. `query_path`, when present (even empty), wins
    /// over every filename. Caller supplies the selection from this exact auth.
    /// # Errors
    /// Invalid metadata/limits, input path budget or pool admission failure.
    pub fn upload_batch(
        &self,
        auth: AuthPermit,
        selection: HttpFileSelection,
        query_path: Option<&str>,
        metadata: BTreeMap<String, String>,
        limits: BatchLimits,
    ) -> Result<UploadBatch, FileStreamError> {
        let limits = limits.validate()?;
        validate_metadata(&metadata).map_err(FileStreamError::Operation)?;
        if let Some(path) = query_path {
            selection
                .check_input(path)
                .map_err(FileStreamError::Context)?;
        }
        let (lease, deadline) = self.admit()?;
        self.upload_batch_reserved(
            Arc::new(auth),
            selection,
            query_path,
            metadata,
            limits,
            Reservation {
                lease,
                deadline,
                settlement: None,
            },
        )
    }

    pub(super) fn upload_batch_reserved(
        &self,
        auth: Arc<AuthPermit>,
        selection: HttpFileSelection,
        query_path: Option<&str>,
        metadata: BTreeMap<String, String>,
        limits: BatchLimits,
        reservation: Reservation,
    ) -> Result<UploadBatch, FileStreamError> {
        let limits = limits.validate()?;
        validate_metadata(&metadata).map_err(FileStreamError::Operation)?;
        if let Some(path) = query_path {
            selection
                .check_input(path)
                .map_err(FileStreamError::Context)?;
        }
        Ok(UploadBatch {
            streams: self.clone(),
            auth,
            reservation,
            selection,
            query_path: query_path.map(str::to_owned),
            metadata,
            limits,
            results: Vec::new(),
            retained: 0,
            received: 0,
            ready: true,
        })
    }
}

/// A failed or cancelled part poisons the entire original request. Completed
/// earlier files remain on disk; neither their writes nor the failed part replay.
pub struct UploadBatch {
    streams: FileStreams,
    auth: Arc<AuthPermit>,
    reservation: Reservation,
    selection: HttpFileSelection,
    query_path: Option<String>,
    metadata: BTreeMap<String, String>,
    limits: BatchLimits,
    results: Vec<UploadResult>,
    retained: usize,
    received: u64,
    ready: bool,
}

impl UploadBatch {
    /// Call for a file part only, with its already decoded full filename.
    /// Unknown form fields are drained by the separately bounded parser.
    /// The returned exclusive borrow prevents overlapping file writes.
    /// # Errors
    /// Failed request, duplicate resolved path, capacity, deadline or open error.
    pub async fn start_file(
        &mut self,
        filename: &str,
        expected: Option<u64>,
    ) -> Result<BatchFile<'_>, FileStreamError> {
        if !self.ready {
            return Err(FileStreamError::Unavailable);
        }
        self.ready = false; // Includes cancellation while opening the file.
        timely(self.reservation.deadline, Ok(()))?;
        if self.results.len() == self.limits.files
            || expected.is_some_and(|n| n > self.limits.total_bytes - self.received)
        {
            return Err(FileStreamError::ResourceExhausted);
        }
        let context = self
            .selection
            .resolve_detailed(self.query_path.as_deref().unwrap_or(filename))
            .map_err(|error| match error {
                crate::files::PathError::Expansion(error) => FileStreamError::PathExpansion(error),
                crate::files::PathError::Context(error) => FileStreamError::Context(error),
            })?;
        if self
            .results
            .iter()
            .any(|result| result.path == context.path)
        {
            // Move retained paths, discard metadata, and keep the batch poisoned.
            // Strings remain within retained_bytes; vector bookkeeping is bounded
            // by files. Duplicate and later targets are never opened.
            let previous = std::mem::take(&mut self.results)
                .into_iter()
                .filter_map(|result| (result.path != context.path).then_some(result.path))
                .collect();
            return Err(FileStreamError::DuplicatePath(
                crate::file_upload::DuplicatePath {
                    path: context.path,
                    previous,
                },
            ));
        }
        let upload = self
            .streams
            .upload_reserved(
                Arc::clone(&self.auth),
                context,
                self.metadata.clone(),
                expected,
                self.reservation.clone(),
            )
            .await?;
        Ok(BatchFile {
            batch: self,
            upload,
        })
    }

    /// Only a valid complete request end permits a result, including zero files.
    /// A multipart decoder must not call this on disconnect or a missing boundary.
    /// # Errors
    /// A failed/cancelled part or an expired original request.
    pub fn finish(self) -> Result<BatchReply, FileStreamError> {
        if !self.ready {
            return Err(FileStreamError::Unavailable);
        }
        timely(self.reservation.deadline, Ok(()))?;
        Ok(BatchReply {
            results: self.results,
            reservation: self.reservation,
        })
    }
}

pub struct BatchFile<'a> {
    batch: &'a mut UploadBatch,
    upload: UploadStream,
}

impl BatchFile<'_> {
    #[must_use]
    pub const fn chunk_bytes(&self) -> usize {
        self.upload.chunk_bytes()
    }

    /// # Errors
    /// Request/file payload budget, actual I/O failure or expired/cancelled work.
    pub async fn write(&mut self, bytes: &[u8]) -> Result<(), FileStreamError> {
        if bytes.len() as u64 > self.batch.limits.total_bytes - self.batch.received {
            self.upload.usable = false;
            self.upload.send.take();
            return Err(FileStreamError::ResourceExhausted);
        }
        self.batch.received += bytes.len() as u64;
        self.upload.write(bytes).await
    }

    /// Complete the current valid part before admitting the next one.
    /// # Errors
    /// Incomplete file, I/O/metadata failure, retained result budget or deadline.
    /// A result-budget failure can follow a fully written file; no rollback.
    pub async fn finish(self) -> Result<(), FileStreamError> {
        let Self { batch, upload } = self;
        let reply = upload.finish().await?;
        let result = reply.result;
        let bytes = result
            .metadata
            .iter()
            .flat_map(|(key, value)| [key.len(), value.len()])
            .try_fold(result.path.len(), usize::checked_add)
            .ok_or(FileStreamError::ResourceExhausted)?;
        if bytes > batch.limits.retained_bytes - batch.retained {
            return Err(FileStreamError::ResourceExhausted);
        }
        batch.retained += bytes;
        batch.results.push(result);
        batch.ready = true;
        Ok(())
    }
}

pub struct BatchReply {
    results: Vec<UploadResult>,
    reservation: Reservation,
}

impl BatchReply {
    // A completed result retains capacity, but is not an active file/decoder.
    // Keep the reservation private to the batch module.
    pub(super) fn release_result_receipt(&mut self) {
        self.reservation.settlement.take();
    }

    #[must_use]
    pub fn results(&self) -> &[UploadResult] {
        &self.results
    }

    /// Serialize and send this completed batch under its original reservation.
    /// Initialization may already proceed after completed file I/O; retained
    /// results, body and all byte clones continue to consume the original slot.
    /// # Errors
    /// Invalid response budgets, escaped payload limit, original expiry or a
    /// lost worker. File effects remain completed even if no reply is delivered.
    pub async fn into_http_response(
        self,
        limits: crate::upload_response::Limits,
    ) -> Result<axum::response::Response, crate::upload_response::Error> {
        crate::upload_response::success(
            self.results,
            limits,
            self.reservation.deadline,
            self.reservation.lease,
        )
        .await
    }
}
