//! Outer token rejection, before Basic and Connect method/header processing.
use super::reply::{Error as ReplyError, Limits};
use crate::{
    MetadataSource, ServiceError, Session,
    command_requests::{Admitted, Authenticated, Error, Failure as RequestFailure},
    file_rpc_response,
};
use axum::{
    http::{HeaderMap, Method},
    response::Response,
};
use std::{fmt, sync::Arc};
use tokio::time::Instant;

/// Carries the original denial and method; callers cannot substitute either.
pub struct Failure {
    request: Arc<RequestFailure>,
    head: bool,
}
impl fmt::Debug for Failure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CommandAuthorizationFailure")
            .field("error", &self.error())
            .finish_non_exhaustive()
    }
}
impl Failure {
    #[must_use]
    pub fn error(&self) -> Error {
        self.request.error()
    }
    #[must_use]
    pub fn deadline(&self) -> Instant {
        self.request.deadline()
    }
    /// Only a protected-session denial uses the original outer API JSON.
    /// HEAD still checks the complete payload budget before suppressing bytes.
    /// # Errors
    /// Unclassified original failure, invalid/exceeded limits, expiry or lost
    /// worker. All retain the same observation, absolute deadline and capacity.
    pub async fn into_response(self, limits: Limits) -> Result<Response, ReplyFailure> {
        if self.error() != Error::Authorization(ServiceError::Unauthenticated) {
            return Err(ReplyFailure {
                error: ReplyError::UnclassifiedFault,
                request: self.request,
            });
        }
        file_rpc_response::token::rejected(
            file_rpc_response::Limits {
                wire_bytes: limits.wire_bytes,
                chunk_bytes: limits.chunk_bytes,
            },
            self.deadline(),
            self.request.clone(),
            self.head,
        )
        .await
        .map_err(|failure| ReplyFailure {
            error: match failure.error() {
                file_rpc_response::Error::InvalidConfiguration => ReplyError::InvalidConfiguration,
                file_rpc_response::Error::ResponseLimit => ReplyError::ResponseLimit,
                file_rpc_response::Error::Encoding => ReplyError::Encoding,
                file_rpc_response::Error::DeadlineExceeded => ReplyError::DeadlineExceeded,
                file_rpc_response::Error::Unavailable => ReplyError::Unavailable,
                file_rpc_response::Error::UnclassifiedFault => ReplyError::UnclassifiedFault,
            },
            request: self.request,
        })
    }
}

pub struct ReplyFailure {
    error: ReplyError,
    request: Arc<RequestFailure>,
}
impl ReplyFailure {
    #[must_use]
    pub const fn error(&self) -> ReplyError {
        self.error
    }
    #[must_use]
    pub fn request_error(&self) -> Error {
        self.request.error()
    }
    #[must_use]
    pub fn deadline(&self) -> Instant {
        self.request.deadline()
    }
}
impl fmt::Debug for ReplyFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CommandAuthorizationReplyFailure")
            .field("error", &self.error)
            .finish_non_exhaustive()
    }
}

impl Admitted {
    /// Use the original token/header admission and retain the method for an
    /// outer authorization rejection. Does not poll a body or resolve Basic.
    /// # Errors
    /// The original admission failure, retaining capacity and its deadline.
    pub async fn authenticate_http<M: MetadataSource>(
        self,
        session: &Session<M>,
        method: &Method,
        headers: HeaderMap,
    ) -> Result<Authenticated, Failure> {
        self.authenticate(session, headers)
            .await
            .map_err(|request| Failure {
                request: Arc::new(request),
                head: method == Method::HEAD,
            })
    }
}
