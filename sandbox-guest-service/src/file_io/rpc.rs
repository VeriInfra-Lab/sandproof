//! One already admitted unary request, from account lookup through actual I/O.
//! Parsing and protocol selection remain the caller's. The opaque preflight
//! carries known account rejections into the fixed unary JSON response sender.
use super::{FileDispatchError, FilePool, FileReply, Owner, check_deadline};
use crate::{AccountPool, AccountSource, AuthPermit, ContextError, files::FileSettings};
use axum::http::HeaderMap;
use sandproof_guest_runtime::{
    file_protocol::Request,
    filesystem::{self, Ownership},
};
use std::{fmt, sync::Arc};
use tokio::time::Instant;

mod preflight;
#[cfg(test)]
mod tests;
pub use preflight::{PreparedRpc, ReadyRpc};

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
/// Only observed account/path or mapped I/O failures have fixed responses; other
/// failures imply no HTTP status. No failure implies that I/O can be replayed.
pub struct RpcFailure {
    error: RpcError,
    rejection: Option<crate::file_rpc_response::Rejection>,
    fault: Option<sandproof_guest_runtime::file_protocol::FileFault>,
    deadline: Instant,
    _owner: Owner,
}
impl RpcFailure {
    #[must_use]
    pub const fn fault(&self) -> Option<&sandproof_guest_runtime::file_protocol::FileFault> {
        self.fault.as_ref()
    }

    #[must_use]
    pub const fn rejection(&self) -> Option<&crate::file_rpc_response::Rejection> {
        self.rejection.as_ref()
    }

    /// Send a fixed account/path or observed file failure for selected unary JSON
    /// request under the original request/file slots and deadline.
    /// # Errors
    /// Unmapped context/I/O failures remain classified, or bounded response
    /// preparation fails. No failure permits replaying a file operation.
    pub async fn into_response(
        mut self,
        limits: crate::file_rpc_response::Limits,
    ) -> Result<axum::response::Response, crate::file_rpc_response::Failure> {
        let rejection = self.rejection.take();
        let fault = self.fault.take();
        let deadline = self.deadline;
        let owner: Owner = Arc::new(self);
        match (rejection, fault) {
            (Some(rejection), _) => {
                crate::file_rpc_response::rejected(rejection, limits, deadline, owner).await
            }
            (None, Some(fault)) => {
                crate::file_rpc_response::io_failure(fault, limits, deadline, owner).await
            }
            (None, None) => Err(crate::file_rpc_response::Failure::unclassified(
                deadline, owner,
            )),
        }
    }

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
        let mut rejection = None;
        let mut fault = None;
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
                .map_err(|error| match error {
                    crate::files::RpcContextError::Expansion(error) => {
                        rejection = Some(crate::file_rpc_response::Rejection::PathExpansion(error));
                        RpcError::Context(ContextError::InvalidArgument)
                    }
                    other => RpcError::Context(other.into()),
                })?;
            let limits = self.limits;
            Self::dispatch_observed(auth, lease, deadline, move || {
                filesystem::execute_detailed(
                    &context.request,
                    Ownership {
                        uid: context.owner.uid,
                        gid: context.owner.gid,
                    },
                    limits,
                )
            })
            .await
            .map_err(|failure| {
                fault = failure.fault;
                RpcError::Dispatch(failure.error)
            })
        }
        .await;
        result.map_err(|error| RpcFailure {
            error,
            rejection,
            fault,
            deadline,
            _owner: owner,
        })
    }
}
