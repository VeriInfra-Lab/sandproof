//! File-scoped authorization under one initialized session snapshot.
//! The center/host gateway still requires its own token and is unchanged.
use crate::{AuthPermit, go_json, owned_body::Owner, upload_response};
use axum::{http::StatusCode, response::Response};
use sandproof_guest_runtime::file_query::{Parameter, TextQuery};
use tokio::time::Instant;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Operation {
    Read,
    Write,
}
impl Operation {
    pub(crate) const fn name(self) -> &'static [u8] {
        match self {
            Self::Read => b"read",
            Self::Write => b"write",
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Limits {
    pub headers: crate::file_upload::HeaderLimits,
    /// Total retained decoded path/username/signature bytes. Raw query bytes
    /// and all fields must separately pass the original query binding budget.
    pub query_bytes: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Denial {
    Uninitialized,
    DuplicateToken,
    TokenMismatch,
    MissingSignature,
    InvalidSignature,
    ExpiredSignature,
}
impl Denial {
    #[must_use]
    pub const fn message(self) -> &'static str {
        match self {
            Self::Uninitialized => "envd not initialized",
            Self::DuplicateToken => {
                "unauthorized access, please provide a valid access token or method signing if supported"
            }
            Self::TokenMismatch => "access token present in header but does not match",
            Self::MissingSignature => "missing signature query parameter",
            Self::InvalidSignature => "invalid signature",
            Self::ExpiredSignature => "signature is already expired",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    Denied(Denial),
    InvalidConfiguration,
    ResourceExhausted,
    DeadlineExceeded,
    ClockUnavailable,
    UnsupportedText(Parameter),
    WrongOperation,
}

/// Opaque, non-cloneable capability for this exact decoded path/user and
/// operation. It cannot be converted by callers to general command authority.
pub struct Authorization {
    pub(crate) auth: AuthPermit,
    pub(crate) query: TextQuery,
    pub(crate) operation: Operation,
    pub(crate) deadline: Instant,
    pub(crate) request_owner: Option<Owner>,
}

#[cfg(target_os = "linux")]
pub(crate) struct Origin {
    pub deadline: Instant,
    pub owner: Option<Owner>,
}
impl Authorization {
    #[must_use]
    pub fn path(&self) -> Option<&str> {
        self.query.path.as_deref()
    }
    #[must_use]
    pub fn username(&self) -> Option<&str> {
        self.query.username.as_deref()
    }
    #[must_use]
    pub const fn operation(&self) -> Operation {
        self.operation
    }
    #[must_use]
    pub const fn deadline(&self) -> Instant {
        self.deadline
    }
    #[must_use]
    pub fn defaults(&self) -> &crate::Defaults {
        self.auth.defaults()
    }
    #[cfg(target_os = "linux")]
    pub(crate) fn take(
        self,
        operation: Operation,
    ) -> Result<(AuthPermit, TextQuery, Origin), Error> {
        if self.operation != operation {
            return Err(Error::WrongOperation);
        }
        if Instant::now() >= self.deadline {
            return Err(Error::DeadlineExceeded);
        }
        Ok((
            self.auth,
            self.query,
            Origin {
                deadline: self.deadline,
                owner: self.request_owner,
            },
        ))
    }
}

/// Send only a classified authentication denial, without reflecting secrets,
/// paths or signatures. Caller supplies its original request slot/deadline.
/// # Errors
/// Invalid response budgets, insufficient wire budget or failed delivery.
pub async fn rejected(
    denial: Denial,
    limits: upload_response::Limits,
    deadline: Instant,
    owner: Owner,
) -> Result<Response, upload_response::Error> {
    rejected_with_head(denial, limits, deadline, owner, false).await
}

pub(crate) async fn rejected_with_head(
    denial: Denial,
    limits: upload_response::Limits,
    deadline: Instant,
    owner: Owner,
    head: bool,
) -> Result<Response, upload_response::Error> {
    upload_response::errors::send_error_with_head(
        StatusCode::UNAUTHORIZED,
        limits,
        deadline,
        owner,
        head,
        move || {
            go_json::bounded(limits.wire_bytes as u64, |put| {
                put(b"{\"code\":401,\"message\":\"")?;
                go_json::string_content(denial.message().as_bytes(), put)?;
                put(b"\"}\n")
            })
            .map_err(|_| upload_response::Error::ResponseLimit)
        },
    )
    .await
}
