//! Admitted blocking unary file operations. Cancellation stops waiting, not a
//! dispatched syscall. The worker retains authorization until I/O settles;
//! returned records retain their slot until dropped. No production route yet.
use crate::{AuthPermit, files::FileContext, owned_body::Owner};
use sandproof_guest_runtime::filesystem::{self, FileError, FileLimits, FileResponse, Ownership};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{OwnedSemaphorePermit, Semaphore};
use tokio::time::Instant;

mod rpc;
pub use rpc::{RpcError, RpcFailure, RpcRequest};

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

pub struct FileReply {
    response: FileResponse,
    deadline: Instant,
    _lease: Arc<Lease>,
}
impl std::fmt::Debug for FileReply {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FileReply")
            .field("response", &self.response)
            .finish_non_exhaustive()
    }
}
struct Lease {
    _slot: OwnedSemaphorePermit,
    _owner: Owner,
}
impl FileReply {
    /// Borrow while the retained response remains charged to the pool. Wire
    /// serialization/copies and network buffers need their own admission.
    #[must_use]
    pub const fn response(&self) -> &FileResponse {
        &self.response
    }

    #[must_use]
    pub const fn deadline(&self) -> Instant {
        self.deadline
    }

    /// Send identity JSON using the same effective deadline and request/file
    /// slots, including all encoded body copies. The caller selects protocol.
    /// # Errors
    /// Response preparation can fail after I/O succeeded; never replay it.
    pub async fn into_response(
        self,
        limits: crate::file_rpc_response::Limits,
    ) -> Result<axum::response::Response, crate::file_rpc_response::Failure> {
        let deadline = self.deadline;
        crate::file_rpc_response::send(
            Arc::new(self),
            |reply| reply.response.as_wire(),
            limits,
            deadline,
        )
        .await
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
        let deadline = self.deadline()?;
        let lease = self.reserve(Arc::new(()))?;
        Self::dispatch_admitted(Arc::new(auth), lease, deadline, operation).await
    }

    fn deadline(&self) -> Result<Instant, FileDispatchError> {
        Instant::now()
            .checked_add(self.timeout)
            .ok_or(FileDispatchError::InvalidArgument)
    }

    fn reserve(&self, owner: Owner) -> Result<Arc<Lease>, FileDispatchError> {
        let slot = Arc::clone(&self.slots)
            .try_acquire_owned()
            .map_err(|_| FileDispatchError::ResourceExhausted)?;
        Ok(Arc::new(Lease {
            _slot: slot,
            _owner: owner,
        }))
    }

    async fn dispatch_admitted<F>(
        auth: Arc<AuthPermit>,
        lease: Arc<Lease>,
        deadline: Instant,
        operation: F,
    ) -> Result<FileReply, FileDispatchError>
    where
        F: FnOnce() -> Result<FileResponse, FileError> + Send + 'static,
    {
        check_deadline(deadline)?;
        let task = tokio::task::spawn_blocking(move || {
            // A queued blocking job may start after its request has expired.
            check_deadline(deadline)?;
            let result = operation();
            // Explicit lifetime through the last operation, including errors.
            drop(auth);
            result
                .map(|response| FileReply {
                    response,
                    deadline,
                    _lease: lease,
                })
                .map_err(FileDispatchError::Operation)
        });
        let result = tokio::time::timeout_at(deadline, task)
            .await
            .map_err(|_| FileDispatchError::DeadlineExceeded)?
            .map_err(|_| FileDispatchError::Unavailable)?;
        check_deadline(deadline)?;
        result
    }
}

fn check_deadline(deadline: Instant) -> Result<(), FileDispatchError> {
    if Instant::now() >= deadline {
        Err(FileDispatchError::DeadlineExceeded)
    } else {
        Ok(())
    }
}
