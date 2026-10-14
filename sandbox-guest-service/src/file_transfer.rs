//! Owned streaming file work. This is an admitted component, not a production
//! HTTP listener. Caller-owned input, serialization and transport buffers have
//! separate budgets. Cancellation never rolls back or replays a write.
use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use sandproof_guest_runtime::filesystem::{
    Ownership,
    transfer::{
        DownloadFile, TransferIoError, TransferLimits, UploadError, UploadFile, UploadResult,
    },
};
use tokio::runtime::Handle;
use tokio::sync::{OwnedSemaphorePermit, Semaphore, mpsc, oneshot};
use tokio::time::{Instant, timeout_at};

use crate::{AuthPermit, files::HttpFileContext};

mod batch;
mod download;
pub use download::{DownloadLimits, DownloadRequest, DownloadRequestError};
mod gzip_download;
mod identity_download;
mod upload;
pub use crate::upload_body::{GzipError, GzipLimits, HttpBodyLimits};
pub use batch::{
    BatchFile, BatchLimits, BatchReply, MultipartError, MultipartLimits, UploadBatch,
    UploadBodyError,
};
pub use gzip_download::GzipDownload;
pub use identity_download::IdentityDownloadError;
pub use upload::{
    AuthorizedUpload, UploadHttpError, UploadLimits, UploadRequest, UploadRequestError,
};

#[cfg(test)]
mod tests;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FileStreamError {
    Context(crate::ContextError),
    Operation(TransferIoError),
    UploadFault(sandproof_guest_runtime::file_upload::Fault),
    DuplicatePath(crate::file_upload::DuplicatePath),
    PathExpansion(crate::upload_response::PathExpansion),
    ResourceExhausted,
    DeadlineExceeded,
    Unavailable,
    InvalidArgument,
}

struct Lease {
    _slot: OwnedSemaphorePermit,
    // The pre-auth request slot follows the transfer through workers/results
    // and all byte clones; dropping an HTTP waiter cannot return it early.
    _request: Option<crate::owned_body::Owner>,
}

#[derive(Clone)]
struct Reservation {
    lease: Arc<Lease>,
    deadline: Instant,
    // The HTTP response waits for every actual upload/decoder owner to end.
    // This is a receipt, not an extra quota or permission to release early.
    settlement: Option<tokio::sync::watch::Sender<()>>,
}

/// A pool admits at most `in_flight` requests/retained transfers.
/// Waiting requests never queue for admission. Each worker uses one body chunk;
/// download chunks keep the lease until released, even after cancellation.
#[derive(Clone)]
pub struct FileStreams {
    slots: Arc<Semaphore>,
    limits: TransferLimits,
    timeout: Duration,
}

impl FileStreams {
    /// # Errors
    /// Invalid per-file limits, concurrency or absolute waiting deadline.
    pub fn new(
        in_flight: usize,
        limits: TransferLimits,
        timeout: Duration,
    ) -> Result<Self, FileStreamError> {
        let limits = limits.validate().map_err(FileStreamError::Operation)?;
        if in_flight == 0
            || in_flight > Semaphore::MAX_PERMITS
            || timeout.is_zero()
            || Instant::now().checked_add(timeout).is_none()
        {
            return Err(FileStreamError::InvalidArgument);
        }
        Ok(Self {
            slots: Arc::new(Semaphore::new(in_flight)),
            limits,
            timeout,
        })
    }

    fn admit(&self) -> Result<(Arc<Lease>, Instant), FileStreamError> {
        self.admit_before(None)
    }

    fn admit_before(
        &self,
        original: Option<crate::file_authorization::Origin>,
    ) -> Result<(Arc<Lease>, Instant), FileStreamError> {
        let deadline = Instant::now()
            .checked_add(self.timeout)
            .ok_or(FileStreamError::InvalidArgument)?;
        let deadline = original
            .as_ref()
            .map_or(deadline, |original| original.deadline.min(deadline));
        timely(deadline, Ok(()))?;
        let permit = Arc::clone(&self.slots)
            .try_acquire_owned()
            .map_err(|_| FileStreamError::ResourceExhausted)?;
        Ok((
            Arc::new(Lease {
                _slot: permit,
                _request: original.and_then(|origin| origin.owner),
            }),
            deadline,
        ))
    }

    /// Caller supplies the context resolved under this exact authorization.
    /// The original authorization stays with the worker until actual I/O ends.
    /// Dropping the open waiter cannot free a running filesystem operation.
    /// # Errors
    /// Admission, opening/ownership, length/metadata validation or wait failure.
    pub async fn upload(
        &self,
        auth: AuthPermit,
        context: HttpFileContext,
        metadata: BTreeMap<String, String>,
        expected: Option<u64>,
    ) -> Result<UploadStream, FileStreamError> {
        let (lease, deadline) = self.admit()?;
        self.upload_reserved(
            Arc::new(auth),
            context,
            metadata,
            expected,
            Reservation {
                lease,
                deadline,
                settlement: None,
            },
        )
        .await
    }

    async fn upload_reserved(
        &self,
        auth: Arc<AuthPermit>,
        context: HttpFileContext,
        metadata: BTreeMap<String, String>,
        expected: Option<u64>,
        reservation: Reservation,
    ) -> Result<UploadStream, FileStreamError> {
        let Reservation {
            lease,
            deadline,
            settlement,
        } = reservation;
        let (send, receive) = mpsc::channel(1);
        let (opened, ready) = oneshot::channel();
        let worker_lease = Arc::clone(&lease);
        let limits = self.limits;
        let runtime = Handle::current();
        tokio::task::spawn_blocking(move || {
            if opened.is_closed() || Instant::now() >= deadline {
                return;
            }
            let result = UploadFile::open_detailed(
                context.path,
                Ownership {
                    uid: context.owner.uid,
                    gid: context.owner.gid,
                },
                metadata,
                expected,
                limits,
            );
            let completion = match timely(deadline, result.map_err(upload_error)) {
                Ok(file) => {
                    if opened.send(Ok(())).is_ok() {
                        upload_worker(file, receive, &runtime, deadline, &worker_lease)
                    } else {
                        None
                    }
                }
                Err(e) => {
                    let _ = opened.send(Err(e));
                    None
                }
            };
            // Includes file destruction/metadata and any cancelled waiter's I/O.
            drop(auth);
            drop(worker_lease);
            drop(settlement);
            if let Some((reply, result)) = completion {
                let _ = reply.send(timely(deadline, result));
            }
        });
        wait(deadline, ready).await??;
        Ok(UploadStream {
            send: Some(send),
            lease,
            deadline,
            chunk_bytes: limits.chunk_bytes,
            usable: true,
        })
    }

    /// # Errors
    /// Admission, opening/seek/length checks or initial waiting failure.
    pub async fn download(
        &self,
        auth: AuthPermit,
        context: HttpFileContext,
    ) -> Result<DownloadStream, FileStreamError> {
        let (lease, deadline) = self.admit()?;
        let (send, receive) = mpsc::channel(1);
        let (opened, ready) = oneshot::channel();
        let worker_lease = Arc::clone(&lease);
        let limits = self.limits;
        let runtime = Handle::current();
        tokio::task::spawn_blocking(move || {
            if opened.is_closed() || Instant::now() >= deadline {
                return;
            }
            match timely(
                deadline,
                DownloadFile::open(&context.path, limits).map_err(FileStreamError::Operation),
            ) {
                Ok(file) => {
                    if opened.send(Ok(file.length())).is_ok() {
                        download_worker(file, &send, &runtime, deadline, &worker_lease);
                    }
                }
                Err(e) => {
                    let _ = opened.send(Err(e));
                }
            }
            drop(auth);
            drop(worker_lease);
        });
        let length = wait(deadline, ready).await??;
        Ok(DownloadStream {
            receive,
            _lease: lease,
            length,
            deadline,
            usable: true,
            ended: false,
        })
    }
}

fn upload_error(error: UploadError) -> FileStreamError {
    match error {
        UploadError::Rejected(error) => FileStreamError::Operation(error),
        UploadError::Fault(fault) => FileStreamError::UploadFault(fault),
    }
}

enum UploadMessage {
    Data(Vec<u8>, oneshot::Sender<Result<(), FileStreamError>>),
    Finish(oneshot::Sender<Result<UploadReply, FileStreamError>>),
}

/// No clone/replay interface. After an interrupted write, this handle is
/// poisoned: the caller cannot infer whether the original bytes were written.
pub struct UploadStream {
    send: Option<mpsc::Sender<UploadMessage>>,
    lease: Arc<Lease>,
    deadline: Instant,
    chunk_bytes: usize,
    usable: bool,
}

pub struct UploadReply {
    result: UploadResult,
    _lease: Arc<Lease>,
}

impl UploadReply {
    #[must_use]
    pub const fn result(&self) -> &UploadResult {
        &self.result
    }
}

impl UploadStream {
    /// At most this many bytes can be copied into one admitted write. The
    /// caller's existing slice and its transport frame are not owned here.
    #[must_use]
    pub const fn chunk_bytes(&self) -> usize {
        self.chunk_bytes
    }

    /// # Errors
    /// Oversized data, failed/expired transfer or an unobserved actual write.
    /// Cancelling this future permanently prevents reuse of this handle.
    pub async fn write(&mut self, bytes: &[u8]) -> Result<(), FileStreamError> {
        if !self.usable {
            self.send.take();
            return Err(FileStreamError::Unavailable);
        }
        self.usable = false;
        if Instant::now() >= self.deadline {
            self.send.take();
            return Err(FileStreamError::DeadlineExceeded);
        }
        if bytes.len() > self.chunk_bytes {
            self.send.take();
            return Err(FileStreamError::ResourceExhausted);
        }
        let (reply, result) = oneshot::channel();
        let send = self.send.as_ref().ok_or(FileStreamError::Unavailable)?;
        timeout_at(
            self.deadline,
            send.send(UploadMessage::Data(bytes.to_vec(), reply)),
        )
        .await
        .map_err(|_| FileStreamError::DeadlineExceeded)?
        .map_err(|_| FileStreamError::Unavailable)?;
        match wait(self.deadline, result)
            .await
            .and_then(std::convert::identity)
        {
            Ok(()) => {
                self.usable = true;
                Ok(())
            }
            Err(e) => {
                self.send.take();
                Err(e)
            }
        }
    }

    /// Only call on a complete valid input end. Dropping the handle instead
    /// leaves any partial file and cannot produce a completed upload reply.
    /// # Errors
    /// Interrupted write, body mismatch, metadata failure or lost final reply.
    pub async fn finish(mut self) -> Result<UploadReply, FileStreamError> {
        if !self.usable {
            return Err(FileStreamError::Unavailable);
        }
        timely(self.deadline, Ok(()))?;
        let (reply, result) = oneshot::channel();
        let send = self.send.take().ok_or(FileStreamError::Unavailable)?;
        timeout_at(self.deadline, send.send(UploadMessage::Finish(reply)))
            .await
            .map_err(|_| FileStreamError::DeadlineExceeded)?
            .map_err(|_| FileStreamError::Unavailable)?;
        // Retain the original admission through the result or cancelled wait.
        let response = wait(self.deadline, result).await?;
        drop(self.lease);
        response
    }
}

type UploadCompletion = (
    oneshot::Sender<Result<UploadReply, FileStreamError>>,
    Result<UploadReply, FileStreamError>,
);

fn upload_worker(
    mut file: UploadFile,
    mut messages: mpsc::Receiver<UploadMessage>,
    runtime: &Handle,
    deadline: Instant,
    lease: &Arc<Lease>,
) -> Option<UploadCompletion> {
    loop {
        let Ok(Some(message)) = runtime.block_on(timeout_at(deadline, messages.recv())) else {
            return None;
        };
        if Instant::now() >= deadline {
            return None;
        }
        match message {
            UploadMessage::Data(bytes, reply) => {
                let result = file.write_chunk_detailed(&bytes).map_err(upload_error);
                let result = timely(deadline, result);
                drop(bytes); // Release this allocation BEFORE accepting the next chunk.
                let failed = result.is_err();
                if reply.send(result).is_err() || failed {
                    return None;
                }
            }
            UploadMessage::Finish(reply) => {
                let result = file
                    .finish_detailed()
                    .map(|result| UploadReply {
                        result,
                        _lease: Arc::clone(lease),
                    })
                    .map_err(upload_error);
                let result = timely(deadline, result);
                // Final ack follows actual file/worker-owned resource release.
                return Some((reply, result));
            }
        }
    }
}

enum DownloadMessage {
    Data(DownloadChunk),
    End,
    Failure(FileStreamError),
}

/// A chunk is deliberately not Clone. Its allocation, admission and producer
/// backpressure remain held until it is dropped, even after the stream ends.
pub struct DownloadChunk {
    bytes: Vec<u8>,
    _lease: Arc<Lease>,
    released: Option<oneshot::Sender<()>>,
}

impl DownloadChunk {
    #[must_use]
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }
}

impl AsRef<[u8]> for DownloadChunk {
    fn as_ref(&self) -> &[u8] {
        &self.bytes
    }
}

impl Drop for DownloadChunk {
    fn drop(&mut self) {
        // Actually free the data before unblocking the producer. Merely sending
        // an ack before field destruction could briefly retain two full chunks.
        drop(std::mem::take(&mut self.bytes));
        if let Some(released) = self.released.take() {
            let _ = released.send(());
        }
    }
}

pub struct DownloadStream {
    receive: mpsc::Receiver<DownloadMessage>,
    _lease: Arc<Lease>,
    length: u64,
    deadline: Instant,
    usable: bool,
    ended: bool,
}

impl DownloadStream {
    #[must_use]
    pub const fn length(&self) -> u64 {
        self.length
    }

    /// Release each previous chunk before waiting for another. Only explicit
    /// End returns None; worker loss, truncation and timeouts return errors.
    /// # Errors
    /// File failure, interrupted previous call, unexpected worker exit or expiry.
    pub async fn next(&mut self) -> Result<Option<DownloadChunk>, FileStreamError> {
        if self.ended {
            return Ok(None);
        }
        if !self.usable {
            self.receive.close();
            return Err(FileStreamError::Unavailable);
        }
        self.usable = false;
        let result = timeout_at(self.deadline, self.receive.recv())
            .await
            .map_err(|_| FileStreamError::DeadlineExceeded);
        let result = match timely(self.deadline, result) {
            Ok(message) => message,
            Err(e) => {
                self.receive.close();
                return Err(e);
            }
        };
        match result {
            Some(DownloadMessage::Data(chunk)) => {
                self.usable = true;
                Ok(Some(chunk))
            }
            Some(DownloadMessage::End) => {
                self.ended = true;
                Ok(None)
            }
            Some(DownloadMessage::Failure(e)) => {
                self.receive.close();
                Err(e)
            }
            None => Err(FileStreamError::Unavailable),
        }
    }
}

fn download_worker(
    mut file: DownloadFile,
    send: &mpsc::Sender<DownloadMessage>,
    runtime: &Handle,
    deadline: Instant,
    lease: &Arc<Lease>,
) {
    loop {
        if send.is_closed() {
            return;
        }
        if Instant::now() >= deadline {
            let _ = send.try_send(DownloadMessage::Failure(FileStreamError::DeadlineExceeded));
            return;
        }
        let bytes = match timely(
            deadline,
            file.read_chunk().map_err(FileStreamError::Operation),
        ) {
            Ok(Some(bytes)) => bytes,
            Ok(None) => {
                let _ = send.try_send(DownloadMessage::End);
                return;
            }
            Err(e) => {
                let _ = send.try_send(DownloadMessage::Failure(e));
                return;
            }
        };
        let (released, consumed) = oneshot::channel();
        let chunk = DownloadChunk {
            bytes,
            _lease: Arc::clone(lease),
            released: Some(released),
        };
        if !runtime
            .block_on(timeout_at(
                deadline,
                send.send(DownloadMessage::Data(chunk)),
            ))
            .is_ok_and(|result| result.is_ok())
        {
            return;
        }
        if runtime.block_on(wait(deadline, consumed)).is_err() {
            let _ = send.try_send(DownloadMessage::Failure(FileStreamError::DeadlineExceeded));
            return;
        }
    }
}

async fn wait<T>(deadline: Instant, result: oneshot::Receiver<T>) -> Result<T, FileStreamError> {
    let result = timeout_at(deadline, result)
        .await
        .map_err(|_| FileStreamError::DeadlineExceeded)?
        .map_err(|_| FileStreamError::Unavailable);
    timely(deadline, result)
}

fn timely<T>(deadline: Instant, result: Result<T, FileStreamError>) -> Result<T, FileStreamError> {
    if Instant::now() >= deadline {
        Err(FileStreamError::DeadlineExceeded)
    } else {
        result
    }
}
