//! Bounded command admission and selected identity-JSON decoding. The caller
//! still owns HTTP route/codec/compression/header negotiation. This component
//! is not a public router and never interprets a tag as a control capability.
#[cfg(any(target_os = "linux", test))]
use crate::upload_body::Input;
use crate::{
    AuthPermit, MetadataSource, ServiceError, Session,
    file_upload::{self, HeaderLimits},
    owned_body::Owner,
    upload_body::HttpBodyLimits,
};
#[cfg(any(target_os = "linux", test))]
use axum::body::Body;
use axum::http::HeaderMap;
use sandproof_guest_runtime::protocol::ProtocolError;
#[cfg(any(target_os = "linux", test))]
use sandproof_guest_runtime::protocol::{self, Request};
use std::{fmt, sync::Arc, time::Duration};
use tokio::{
    sync::{OwnedSemaphorePermit, Semaphore},
    time::{Instant, timeout_at},
};

#[cfg(target_os = "linux")]
mod dispatch;
#[cfg(target_os = "linux")]
mod preflight;
#[cfg(target_os = "linux")]
mod start;
#[cfg(target_os = "linux")]
pub use dispatch::Reply;
#[cfg(target_os = "linux")]
pub use preflight::{Prepared, Ready};
#[cfg(target_os = "linux")]
pub use start::StartPreparation;
#[cfg(test)]
mod tests;

#[cfg(any(target_os = "linux", test))]
mod compression;
pub mod http;
pub use crate::request_gzip::CompressionStage;

/// Explicit component budgets. Wire bytes include the five-byte Start/Connect
/// envelope; decoded JSON and input bytes have no larger budget. Socket/header
/// allocation before admission is bounded separately by the listener.
#[derive(Clone, Copy, Debug)]
pub struct Limits {
    pub in_flight: usize,
    pub timeout: Duration,
    pub headers: HeaderLimits,
    pub body: HttpBodyLimits,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AdmissionError {
    InvalidConfiguration,
    ResourceExhausted,
}
#[derive(Clone)]
pub struct CommandRequests {
    slots: Arc<Semaphore>,
    limits: Limits,
}
impl CommandRequests {
    /// # Errors
    /// Invalid or unrepresentable budgets; no tasks or commands are created.
    pub fn new(limits: Limits) -> Result<Self, AdmissionError> {
        if limits.in_flight == 0
            || limits.in_flight > Semaphore::MAX_PERMITS
            || limits.timeout.is_zero()
            || Instant::now().checked_add(limits.timeout).is_none()
            || file_upload::check_headers(&HeaderMap::new(), limits.headers).is_err()
            || limits.body.validate().is_err()
            || limits.body.total_bytes == 0
            || limits.body.total_bytes > (isize::MAX as usize / 16) as u64
        {
            return Err(AdmissionError::InvalidConfiguration);
        }
        Ok(Self {
            slots: Arc::new(Semaphore::new(limits.in_flight)),
            limits,
        })
    }

    /// Reserve before token/account waits or body polling. Never queues.
    /// # Errors
    /// Full capacity or an unrepresentable original absolute deadline.
    pub fn admit(&self) -> Result<Admitted, AdmissionError> {
        let deadline = Instant::now()
            .checked_add(self.limits.timeout)
            .ok_or(AdmissionError::InvalidConfiguration)?;
        let slot = self
            .slots
            .clone()
            .try_acquire_owned()
            .map_err(|_| AdmissionError::ResourceExhausted)?;
        Ok(Admitted {
            limits: self.limits,
            deadline,
            owner: Arc::new(Lease { _slot: slot }),
        })
    }
}
struct Lease {
    _slot: OwnedSemaphorePermit,
}
pub struct Admitted {
    limits: Limits,
    deadline: Instant,
    owner: Owner,
}
impl Admitted {
    #[must_use]
    pub const fn deadline(&self) -> Instant {
        self.deadline
    }

    /// Check all parsed headers before waiting for one protected auth/defaults
    /// snapshot. Neither body decoding nor Basic account lookup happens here.
    /// # Errors
    /// Header budget, token/session rejection or expiry. A failure keeps the
    /// request slot, but no completed authentication read lock.
    pub async fn authenticate<M: MetadataSource>(
        self,
        session: &Session<M>,
        headers: HeaderMap,
    ) -> Result<Authenticated, Failure> {
        let result = async {
            check_deadline(self.deadline)?;
            file_upload::check_headers(&headers, self.limits.headers)
                .map_err(|_| Error::HeaderLimit)?;
            let auth = timeout_at(self.deadline, session.authorize_headers(&headers))
                .await
                .map_err(|_| Error::DeadlineExceeded)?
                .map_err(Error::Authorization)?;
            check_deadline(self.deadline)?;
            Ok(auth)
        }
        .await;
        match result {
            Ok(auth) => Ok(Authenticated {
                admitted: self,
                auth,
                headers,
            }),
            Err(error) => Err(Failure::new(error, self.deadline, self.owner)),
        }
    }
}

pub struct Authenticated {
    admitted: Admitted,
    auth: AuthPermit,
    headers: HeaderMap,
}
impl Authenticated {
    #[must_use]
    pub const fn deadline(&self) -> Instant {
        self.admitted.deadline
    }
    #[must_use]
    pub const fn headers(&self) -> &HeaderMap {
        &self.headers
    }
    #[must_use]
    pub const fn authorization(&self) -> &AuthPermit {
        &self.auth
    }

    // Only the Linux Basic preflight can produce a public Ready request.
    #[cfg(any(target_os = "linux", test))]
    async fn decode(
        self,
        method: &'static str,
        body: Body,
        deadline: Instant,
    ) -> Result<Decoded, Failure> {
        self.decode_with(body, deadline, move |bytes, limit, _, _| {
            protocol::decode_request(method, bytes, limit).map_err(Error::Protocol)
        })
        .await
    }

    #[cfg(any(target_os = "linux", test))]
    async fn decode_with<F>(
        self,
        body: Body,
        deadline: Instant,
        decode: F,
    ) -> Result<Decoded, Failure>
    where
        F: FnOnce(
                &[u8],
                usize,
                Instant,
                &tokio::sync::watch::Receiver<bool>,
            ) -> Result<Request, Error>
            + Send
            + 'static,
    {
        use axum::body::HttpBody;
        let deadline = deadline.min(self.deadline());
        let owner = self.admitted.owner.clone();
        let result = async {
            check_deadline(deadline)?;
            let expected = body.size_hint().exact();
            let mut input = Input::new(body, expected, self.admitted.limits.body, deadline)
                .map_err(body_error)?;
            let limit = usize::try_from(self.admitted.limits.body.total_bytes)
                .map_err(|_| Error::BodyLimit)?;
            let mut payload = Vec::new();
            while let Some(bytes) = input.next().await.map_err(body_error)? {
                let required = payload.len() + bytes.len(); // Input bounds sum.
                if required > payload.capacity() {
                    let capacity = payload
                        .capacity()
                        .saturating_mul(2)
                        .max(required)
                        .min(limit);
                    payload
                        .try_reserve_exact(capacity - payload.len())
                        .map_err(|_| Error::BodyLimit)?;
                }
                payload.extend_from_slice(&bytes);
            }
            drop(input);
            check_deadline(deadline)?;
            let (cancel, cancelled) = tokio::sync::watch::channel(false);
            let task = tokio::task::spawn_blocking(move || {
                crate::owned_body::check_active::<Error>(deadline, &cancelled)?;
                let request = decode(&payload, limit, deadline, &cancelled)?;
                drop(payload);
                crate::owned_body::check_active::<Error>(deadline, &cancelled)?;
                Ok::<_, Error>(Decoded {
                    request,
                    authenticated: self,
                    deadline,
                })
            });
            let decoded = timeout_at(deadline, task)
                .await
                .map_err(|_| Error::DeadlineExceeded)?
                .map_err(|_| Error::Unavailable)??;
            drop(cancel);
            check_deadline(deadline)?;
            Ok(decoded)
        }
        .await;
        result.map_err(|error| Failure::new(error, deadline, owner))
    }
}
#[cfg(any(target_os = "linux", test))]
struct Decoded {
    request: Request,
    authenticated: Authenticated,
    deadline: Instant,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    DeadlineExceeded,
    HeaderLimit,
    Authorization(ServiceError),
    BodyLimit,
    Transport,
    Protocol(ProtocolError),
    Compression {
        stage: CompressionStage,
        error: crate::upload_body::GzipError,
    },
    Unavailable,
    #[cfg(target_os = "linux")]
    Context(crate::ContextError),
    #[cfg(target_os = "linux")]
    Registry(crate::command::RegistryError),
    #[cfg(target_os = "linux")]
    Registration(crate::command::RegistrationError),
    #[cfg(target_os = "linux")]
    Command(crate::command::CommandError),
    #[cfg(target_os = "linux")]
    Response(crate::command::CommandBodyError),
    /// Wrong dispatch entry or a tag requiring the unselected tag policy.
    UnsupportedOperation,
}
impl From<ProtocolError> for Error {
    fn from(error: ProtocolError) -> Self {
        Self::Protocol(error)
    }
}
impl crate::owned_body::Failure for Error {
    fn deadline() -> Self {
        Self::DeadlineExceeded
    }
    fn unavailable() -> Self {
        Self::Unavailable
    }
}
impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("command request failed")
    }
}
impl std::error::Error for Error {}
pub struct Failure {
    error: Error,
    deadline: Instant,
    #[cfg(target_os = "linux")]
    unknown_user: Option<crate::upload_response::UnknownUser>,
    #[cfg(target_os = "linux")]
    default_account: Option<http::default_account::Rejection>,
    #[cfg(target_os = "linux")]
    operation: Option<http::operation::Rejection>,
    #[cfg(target_os = "linux")]
    response: Option<http::ResponseFormat>,
    _owner: Owner,
}
impl Failure {
    fn new(error: Error, deadline: Instant, owner: Owner) -> Self {
        Self {
            error,
            deadline,
            #[cfg(target_os = "linux")]
            unknown_user: None,
            #[cfg(target_os = "linux")]
            default_account: None,
            #[cfg(target_os = "linux")]
            operation: None,
            #[cfg(target_os = "linux")]
            response: None,
            _owner: owner,
        }
    }
    #[must_use]
    pub const fn error(&self) -> Error {
        self.error
    }
    #[must_use]
    pub const fn deadline(&self) -> Instant {
        self.deadline
    }
}
impl fmt::Debug for Failure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CommandRequestFailure")
            .field("error", &self.error)
            .finish_non_exhaustive()
    }
}
#[cfg(any(target_os = "linux", test))]
fn body_error(error: crate::upload_body::GzipError) -> Error {
    use crate::upload_body::GzipError;
    match error {
        GzipError::DeadlineExceeded => Error::DeadlineExceeded,
        GzipError::InputLimit => Error::BodyLimit,
        _ => Error::Transport,
    }
}
fn check_deadline(deadline: Instant) -> Result<(), Error> {
    if Instant::now() >= deadline {
        Err(Error::DeadlineExceeded)
    } else {
        Ok(())
    }
}
