use super::{Limits, ReplyFailure, Snapshot, Source, send};
use crate::{
    AccountPool, AccountSource, ContextLimits,
    command_requests::{Authenticated, Error, Failure as RequestFailure, Prepared},
};
use axum::{http::Method, response::Response};
use std::{fmt, sync::Arc};
use tokio::time::Instant;

/// Opaque handoff: the original lookup failure and original HTTP metadata cannot
/// be replaced before emitting the observed explicit-account response.
pub struct Failure {
    request: RequestFailure,
    snapshot: Snapshot,
}
impl fmt::Debug for Failure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CommandAccountFailure")
            .field("error", &self.request.error())
            .finish_non_exhaustive()
    }
}
impl Failure {
    #[must_use]
    pub const fn error(&self) -> Error {
        self.request.error()
    }
    #[must_use]
    pub const fn deadline(&self) -> Instant {
        self.request.deadline()
    }
    /// Only a bounded missing explicit account has a public response here.
    /// Queued/cancelled/failed lookups never become a fabricated unknown user.
    /// # Errors
    /// Unclassified source/protocol, invalid/exceeded budgets, expiry or worker
    /// loss, retaining the original observation, request capacity and deadline.
    pub async fn into_response(mut self, limits: Limits) -> Result<Response, ReplyFailure> {
        let source = Source {
            user: self.request.unknown_user.take(),
            request_error: self.request.error(),
            snapshot: self.snapshot,
            deadline: self.request.deadline(),
            _owner: Arc::new(self.request),
        };
        send(source, limits).await
    }
}
impl Authenticated {
    /// Resolve Basic using the original account preflight, preserving only the
    /// same first content-type/version and method for a possible authn reply.
    /// Header interpretation happens after the account failure, not before it.
    /// # Errors
    /// The original account/preflight failures, with their unchanged deadline.
    pub async fn prepare_http<S: AccountSource>(
        self,
        method: &Method,
        accounts: &AccountPool<S>,
        limits: ContextLimits,
    ) -> Result<Prepared, Failure> {
        let snapshot =
            Snapshot::capture(self.headers(), method, self.admitted.limits.headers.bytes);
        self.prepare(accounts, limits)
            .await
            .map_err(|request| Failure { request, snapshot })
    }
}
