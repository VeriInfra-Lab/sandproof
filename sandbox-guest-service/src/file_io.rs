//! Admitted blocking unary file operations. Cancellation stops waiting, not a
//! dispatched syscall. The worker retains authorization until I/O settles;
//! returned records retain their slot until dropped. No production route yet.
use crate::{AuthPermit, files::FileContext};
use sandproof_guest_runtime::filesystem::{self, FileError, FileLimits, FileResponse, Ownership};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

#[cfg(test)]
mod tests;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FileDispatchError {
    Operation(FileError),
    ResourceExhausted,
    DeadlineExceeded,
    Unavailable,
    InvalidArgument,
}

#[derive(Debug)]
pub struct FileReply {
    response: FileResponse,
    _slot: OwnedSemaphorePermit,
}
impl FileReply {
    /// Borrow while the retained response remains charged to the pool. Wire
    /// serialization/copies and network buffers need their own admission.
    #[must_use]
    pub const fn response(&self) -> &FileResponse {
        &self.response
    }
}

#[derive(Clone)]
pub struct FilePool {
    slots: Arc<Semaphore>,
    limits: FileLimits,
    timeout: Duration,
}
impl FilePool {
    /// These are component settings; production budgets require E01 review.
    /// # Errors
    /// Invalid concurrency, per-operation budgets or waiting deadline.
    pub fn new(
        in_flight: usize,
        limits: FileLimits,
        timeout: Duration,
    ) -> Result<Self, FileDispatchError> {
        let limits = limits
            .validate()
            .map_err(|_| FileDispatchError::InvalidArgument)?;
        if in_flight == 0
            || in_flight > Semaphore::MAX_PERMITS
            || timeout.is_zero()
            || tokio::time::Instant::now().checked_add(timeout).is_none()
        {
            return Err(FileDispatchError::InvalidArgument);
        }
        Ok(Self {
            slots: Arc::new(Semaphore::new(in_flight)),
            limits,
            timeout,
        })
    }

    /// Caller supplies context resolved with this same authorization snapshot.
    /// Files use daemon I/O credentials; ownership only selects new directory
    /// ownership. No automatic retry or rollback after a lost/timed-out reply.
    /// # Errors
    /// Refused admission, operation error, unavailable worker or wait deadline.
    pub async fn execute(
        &self,
        auth: AuthPermit,
        context: FileContext,
    ) -> Result<FileReply, FileDispatchError> {
        let limits = self.limits;
        self.dispatch(auth, move || {
            filesystem::execute(
                &context.request,
                Ownership {
                    uid: context.owner.uid,
                    gid: context.owner.gid,
                },
                limits,
            )
        })
        .await
    }

    async fn dispatch<F>(
        &self,
        auth: AuthPermit,
        operation: F,
    ) -> Result<FileReply, FileDispatchError>
    where
        F: FnOnce() -> Result<FileResponse, FileError> + Send + 'static,
    {
        let slot = Arc::clone(&self.slots)
            .try_acquire_owned()
            .map_err(|_| FileDispatchError::ResourceExhausted)?;
        let task = tokio::task::spawn_blocking(move || {
            let result = operation();
            // Explicit lifetime through the last operation, including errors.
            drop(auth);
            result
                .map(|response| FileReply {
                    response,
                    _slot: slot,
                })
                .map_err(FileDispatchError::Operation)
        });
        tokio::time::timeout(self.timeout, task)
            .await
            .map_err(|_| FileDispatchError::DeadlineExceeded)?
            .map_err(|_| FileDispatchError::Unavailable)?
    }
}
