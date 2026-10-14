//! One already admitted unary request, from account lookup through actual I/O.
//! Parsing, authentication and wire error/response policy remain the caller's.
use super::{FileDispatchError, FilePool, FileReply, Owner, check_deadline};
use crate::{AccountPool, AccountSource, AuthPermit, ContextError, files::FileSettings};
use axum::http::HeaderMap;
use sandproof_guest_runtime::{
    file_protocol::Request,
    filesystem::{self, Ownership},
};
use std::{fmt, sync::Arc};
use tokio::time::Instant;

#[cfg(test)]
mod tests;

/// An authenticated, decoded and bounded request plus its original admission.
/// Only the trusted caller creates this envelope; it must reserve the request
/// budget before body collection and retain wire response allocations itself.
pub struct RpcRequest {
    pub auth: AuthPermit,
    pub request: Request,
    pub headers: HeaderMap,
    pub deadline: Instant,
    pub owner: Owner,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RpcError {
    Context(ContextError),
    Dispatch(FileDispatchError),
}

/// Retains the request responsibility and, once reserved, the unary file slot.
/// A timed-out/cancelled blocking worker can independently hold both as well.
/// No HTTP status or retry instruction is implied by this classified failure.
pub struct RpcFailure {
    error: RpcError,
    deadline: Instant,
    _owner: Owner,
}
impl RpcFailure {
    #[must_use]
    pub const fn error(&self) -> RpcError {
        self.error
    }
    #[must_use]
    pub const fn deadline(&self) -> Instant {
        self.deadline
    }
}
impl fmt::Debug for RpcFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("FileRpcFailure")
            .field("error", &self.error)
            .finish_non_exhaustive()
    }
}

impl FilePool {
    /// Resolve and dispatch under one pool reservation and absolute deadline.
    /// Basic account/default/path rules remain the shared original resolver.
    /// Move resolves both paths from one snapshot before any file side effect.
    /// # Errors
    /// Refused admission, context failure, actual file error or lost/expired
    /// work. Cancellation/expiry never rolls back or automatically replays I/O.
    pub async fn execute_rpc<S: AccountSource>(
        &self,
        request: RpcRequest,
        accounts: &AccountPool<S>,
        settings: &FileSettings,
    ) -> Result<FileReply, RpcFailure> {
        let mut deadline = request.deadline;
        let mut owner = request.owner;
        let result = async {
            deadline = deadline.min(self.deadline().map_err(RpcError::Dispatch)?);
            check_deadline(deadline).map_err(RpcError::Dispatch)?;
            let lease = self
                .reserve(Arc::clone(&owner))
                .map_err(RpcError::Dispatch)?;
            owner = lease.clone();
            let auth = Arc::new(request.auth);
            let context = auth
                .resolve_file_rpc_admitted(
                    &request.request,
                    &request.headers,
                    accounts,
                    settings,
                    deadline,
                    Arc::new((Arc::clone(&auth), Arc::clone(&lease))),
                )
                .await
                .map_err(RpcError::Context)?;
            let limits = self.limits;
            Self::dispatch_admitted(auth, lease, deadline, move || {
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
            .map_err(RpcError::Dispatch)
        }
        .await;
        result.map_err(|error| RpcFailure {
            error,
            deadline,
            _owner: owner,
        })
    }
}
