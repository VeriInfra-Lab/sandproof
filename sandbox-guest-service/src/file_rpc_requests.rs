//! Unary file admission, token wait and bounded identity/gzip JSON decoding.
//! This is a component of the future RPC handler, not protocol negotiation or
//! Basic account middleware. Linux `FilePool::prepare_rpc` supplies the explicit
//! account preflight; the caller must still select the supported route/codec.
use crate::task_accounting::{self, BlockingTask};
use crate::{
    AuthPermit, MetadataSource, ServiceError, Session,
    file_upload::{self, HeaderLimits},
    owned_body::{self, Owner},
    upload_body::{GzipError, HttpBodyLimits, Input},
};
use axum::{
    body::{Body, HttpBody},
    http::HeaderMap,
};
use sandproof_guest_runtime::{
    file_protocol::{self, Request},
    protocol::ProtocolError,
};
use std::{fmt, sync::Arc, time::Duration};
use tokio::{
    sync::{OwnedSemaphorePermit, Semaphore, watch},
    time::{Instant, timeout_at},
};

mod compression;
mod selection;
use crate::connect_timeout as timeout;
pub use compression::CompressionStage;
mod version;
pub use selection::{SelectionError, UnsupportedCompression};
#[cfg(any(target_os = "linux", test))]
pub(crate) use selection::{response_encoding, select as select_http};
pub use timeout::{InvalidTimeout, TimeoutReason};
pub use version::InvalidVersion;
#[cfg(test)]
pub(crate) mod tests;

/// No production defaults. Capacity includes retained headers, accepted frames,
/// collected wire bytes, expanded JSON, decoding work, decoded records and
/// downstream response work. Wire and expanded payloads have the same separate cap.
/// Socket/header parsing and allocations before this boundary are separate.
#[derive(Clone, Copy, Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
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
pub struct FileRpcRequests {
    slots: Arc<Semaphore>,
    limits: Limits,
}
impl FileRpcRequests {
    /// # Errors
    /// Zero, unrepresentable or inconsistent component budgets.
    pub fn new(limits: Limits) -> Result<Self, AdmissionError> {
        if limits.in_flight == 0
            || limits.in_flight > Semaphore::MAX_PERMITS
            || limits.timeout.is_zero()
            || Instant::now().checked_add(limits.timeout).is_none()
            || file_upload::check_headers(&HeaderMap::new(), limits.headers).is_err()
            || limits.body.validate().is_err()
            || limits.body.total_bytes == 0
            || usize::try_from(limits.body.total_bytes).is_err()
            || limits.body.total_bytes > (isize::MAX as usize / 16) as u64
        {
            return Err(AdmissionError::InvalidConfiguration);
        }
        Ok(Self {
            slots: Arc::new(Semaphore::new(limits.in_flight)),
            limits,
        })
    }

    /// Reserve before authorization, reading or decoding. There is no queue.
    /// # Errors
    /// Saturation or an unrepresentable absolute deadline. Unadmitted failures
    /// must use the listener's separate bounded rejection/connection capacity.
    pub fn admit(&self) -> Result<Admitted, AdmissionError> {
        let deadline = Instant::now()
            .checked_add(self.limits.timeout)
            .ok_or(AdmissionError::InvalidConfiguration)?;
        let slot = Arc::clone(&self.slots)
            .try_acquire_owned()
            .map_err(|_| AdmissionError::ResourceExhausted)?;
        Ok(Admitted {
            lease: Arc::new(Lease { _slot: slot }),
            limits: self.limits,
            deadline,
        })
    }
}
struct Lease {
    _slot: OwnedSemaphorePermit,
}
pub struct Admitted {
    lease: Owner,
    limits: Limits,
    deadline: Instant,
}
impl Admitted {
    #[cfg(target_os = "linux")]
    pub(crate) fn owner(&self) -> Owner {
        self.lease.clone()
    }
    #[must_use]
    pub const fn deadline(&self) -> Instant {
        self.deadline
    }

    /// Check every parsed header before waiting for the protected token snapshot.
    /// Does not read the body, parse Connect headers or look up Basic accounts.
    /// # Errors
    /// Expiry, header budget or protected-session rejection. Failure retains
    /// the admitted request slot but does not keep a completed auth read lock.
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
            Err(error) => Err(Failure {
                error,
                deadline: self.deadline,
                _owner: self.lease,
            }),
        }
    }
}

/// The middleware phase remains separate from body decoding. Account and
/// protocol middleware must preserve this admission and its original deadline.
pub struct Authenticated {
    admitted: Admitted,
    auth: AuthPermit,
    headers: HeaderMap,
}
impl Authenticated {
    #[cfg(any(target_os = "linux", test))]
    pub(crate) fn json_codec(&self) -> Option<file_protocol::JsonContentType> {
        let value = self.headers.get("content-type")?;
        file_protocol::json_content_type(
            std::str::from_utf8(value.as_bytes()).ok()?,
            self.admitted.limits.headers.bytes,
        )
        .ok()
        .flatten()
    }

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

    #[cfg(target_os = "linux")]
    pub(crate) fn owner(&self) -> Owner {
        self.admitted.lease.clone()
    }

    #[cfg(target_os = "linux")]
    pub(crate) fn retain(&mut self, owner: Owner, deadline: Instant) {
        self.admitted.lease = owner;
        self.admitted.deadline = self.admitted.deadline.min(deadline);
    }

    /// Decode a selected POST unary JSON file method after Basic middleware and
    /// HTTP/codec selection. Compression is checked here before version errors.
    /// The trusted HTTP framing layer supplies the unconsumed Body; an exact
    /// size hint is its length, never a reparse of Content-Length. Applies the
    /// fixed Connect timeout now, capped by the admission's original deadline.
    /// # Errors
    /// Unsupported compression, invalid version/timeout, body framing/transport,
    /// malformed JSON, gzip header/body, unknown method, expiry or lost decoder.
    /// No file effects occur here.
    pub async fn decode(self, method: &str, body: Body) -> Result<Decoded, Failure> {
        let mut deadline = self.admitted.deadline;
        let owner: Owner = self.admitted.lease.clone();
        let result = async {
            check_deadline(deadline)?;
            // Connect sets a valid timeout before NewConn, then reports unknown
            // compression before version and timeout parse failures. No body read.
            let timeout = timeout::deadline(&self.headers, Instant::now(), deadline);
            if let Ok(end) = timeout {
                deadline = end;
            }
            selection::check_compression(&self.headers).map_err(Error::UnsupportedCompression)?;
            version::check(&self.headers).map_err(Error::Version)?;
            deadline = timeout.map_err(Error::Timeout)?;
            check_deadline(deadline)?;
            let method = match method {
                "Stat" => "Stat",
                "MakeDir" => "MakeDir",
                "ListDir" => "ListDir",
                "Move" => "Move",
                "Remove" => "Remove",
                _ => return Err(Error::Protocol(ProtocolError::Unimplemented)),
            };
            let expected = body.size_hint().exact();
            let mut input = Input::new(body, expected, self.admitted.limits.body, deadline)
                .map_err(body_error)?;
            // new() established representability and a positive collection cap.
            let limit = usize::try_from(self.admitted.limits.body.total_bytes)
                .map_err(|_| Error::Unavailable)?;
            let mut payload = Vec::new();
            while let Some(bytes) = input.next().await.map_err(body_error)? {
                let required = payload.len() + bytes.len(); // Input bounds their sum.
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
                // Release the source chunk before demanding another frame.
            }
            drop(input);
            check_deadline(deadline)?;
            let compressed = selection::is_gzip_request(&self.headers);
            let (signal, cancelled) = watch::channel(false);
            let task = task_accounting::spawn_blocking(BlockingTask::FileDecode, move || {
                owned_body::check_active::<Error>(deadline, &cancelled)?;
                if compressed {
                    payload = compression::expand(
                        &payload,
                        limit,
                        self.admitted.limits.body.frame_bytes,
                        deadline,
                        &cancelled,
                    )?;
                }
                let request = file_protocol::decode_request_observed(method, &payload, limit)
                    .map_err(|failure| match failure {
                        file_protocol::DecodeFailure::Json(error) => Error::Json(error),
                        file_protocol::DecodeFailure::Protocol(error) => Error::Protocol(error),
                    })?;
                drop(payload);
                owned_body::check_active::<Error>(deadline, &cancelled)?;
                Ok(Decoded {
                    request,
                    authenticated: self,
                    deadline,
                })
            });
            let decoded = timeout_at(deadline, task)
                .await
                .map_err(|_| Error::DeadlineExceeded)?
                .map_err(|_| Error::Unavailable)??;
            drop(signal); // Cancellation/timeout also drops it; worker retains self.
            check_deadline(deadline)?;
            Ok(decoded)
        }
        .await;
        result.map_err(|error| Failure {
            error,
            deadline,
            _owner: owner,
        })
    }
}

/// Bounded decoded fields, one authorization snapshot and the original slot.
/// Keeping this result keeps authorization; I/O later releases it on settlement.
pub struct Decoded {
    request: Request,
    authenticated: Authenticated,
    deadline: Instant,
}
impl Decoded {
    #[must_use]
    pub const fn request(&self) -> &Request {
        &self.request
    }
    #[must_use]
    pub const fn deadline(&self) -> Instant {
        self.deadline
    }
    #[must_use]
    pub const fn headers(&self) -> &HeaderMap {
        &self.authenticated.headers
    }
    #[must_use]
    pub const fn authorization(&self) -> &AuthPermit {
        &self.authenticated.auth
    }

    /// Consume the envelope without replacing decoded fields, authorization or
    /// the original admission/deadline before entering the file worker pool.
    #[cfg(target_os = "linux")]
    #[must_use]
    pub fn into_rpc(self) -> crate::file_io::RpcRequest {
        crate::file_io::RpcRequest {
            auth: self.authenticated.auth,
            request: self.request,
            headers: self.authenticated.headers,
            deadline: self.deadline,
            owner: self.authenticated.admitted.lease,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Error {
    DeadlineExceeded,
    HeaderLimit,
    Authorization(ServiceError),
    Version(InvalidVersion),
    Timeout(InvalidTimeout),
    UnsupportedCompression(UnsupportedCompression),
    BodyLimit,
    Transport,
    Protocol(ProtocolError),
    Json(file_protocol::JsonError),
    Compression {
        stage: CompressionStage,
        error: GzipError,
    },
    Unavailable,
}
impl owned_body::Failure for Error {
    fn deadline() -> Self {
        Self::DeadlineExceeded
    }
    fn unavailable() -> Self {
        Self::Unavailable
    }
}
impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("file RPC request failed")
    }
}
impl std::error::Error for Error {}
/// Actual token denials and observed compression/version/timeout/JSON and known
/// gzip errors have fixed responses.
/// Other classifications remain unmapped; no automatic replay.
pub struct Failure {
    error: Error,
    deadline: Instant,
    _owner: Owner,
}
impl Failure {
    fn gzip_rejection(&self) -> Option<crate::file_rpc_response::GzipRejection> {
        match self.error {
            Error::Compression { stage, error } => {
                crate::file_rpc_response::GzipRejection::observed(stage, error)
            }
            _ => None,
        }
    }
    #[cfg(any(target_os = "linux", test))]
    pub(crate) fn has_response(&self) -> bool {
        matches!(
            self.error,
            Error::Authorization(ServiceError::Unauthenticated)
                | Error::Version(_)
                | Error::Timeout(_)
                | Error::UnsupportedCompression(_)
                | Error::Json(_)
        ) || self.gzip_rejection().is_some()
    }
    #[must_use]
    pub const fn error(&self) -> &Error {
        &self.error
    }
    #[must_use]
    pub const fn deadline(&self) -> Instant {
        self.deadline
    }

    /// Consume a failure and its original capacity. Actual token denials use
    /// the outer API JSON format; selected POST unary JSON protocol errors use
    /// the Connect JSON format. Both use identity response encoding.
    /// # Errors
    /// Unmapped source failure, response budget, original expiry or lost worker.
    /// Every failure retains the original owner; no fresh deadline is granted.
    pub async fn into_response(
        self,
        limits: crate::file_rpc_response::Limits,
    ) -> Result<axum::response::Response, crate::file_rpc_response::Failure> {
        use crate::file_rpc_response::{self, Rejection};
        if matches!(
            self.error,
            Error::Authorization(ServiceError::Unauthenticated)
        ) {
            return self.into_authorization_response(limits, false).await;
        }
        let gzip = self.gzip_rejection();
        let Self {
            error,
            deadline,
            _owner: owner,
        } = self;
        let rejection = match (error, gzip) {
            (Error::Version(error), _) => Rejection::Version(error),
            (Error::Timeout(error), _) => Rejection::Timeout(error),
            (Error::UnsupportedCompression(error), _) => Rejection::UnsupportedCompression(error),
            (Error::Json(error), _) => Rejection::Json(error),
            (_, Some(error)) => Rejection::Gzip(error),
            (error, None) => {
                return Err(file_rpc_response::Failure::unclassified(
                    deadline,
                    Arc::new((error, owner)),
                ));
            }
        };
        file_rpc_response::rejected(rejection, limits, deadline, owner).await
    }

    /// Only the actual protected-session denial uses this pre-protocol format.
    /// HEAD checks the full payload budget before suppressing its body.
    pub(crate) async fn into_authorization_response(
        self,
        limits: crate::file_rpc_response::Limits,
        head: bool,
    ) -> Result<axum::response::Response, crate::file_rpc_response::Failure> {
        let Self {
            error,
            deadline,
            _owner: owner,
        } = self;
        if error != Error::Authorization(ServiceError::Unauthenticated) {
            return Err(crate::file_rpc_response::Failure::unclassified(
                deadline,
                Arc::new((error, owner)),
            ));
        }
        crate::file_rpc_response::token::rejected(limits, deadline, owner, head).await
    }
}
impl fmt::Debug for Failure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("FileRpcRequestFailure")
            .field("error", &self.error)
            .finish_non_exhaustive()
    }
}
fn check_deadline(deadline: Instant) -> Result<(), Error> {
    if Instant::now() >= deadline {
        Err(Error::DeadlineExceeded)
    } else {
        Ok(())
    }
}
fn body_error(error: GzipError) -> Error {
    match error {
        GzipError::InputLimit => Error::BodyLimit,
        GzipError::DeadlineExceeded => Error::DeadlineExceeded,
        // Body errors remain opaque, including arbitrary wrapped source errors.
        _ => Error::Transport,
    }
}
