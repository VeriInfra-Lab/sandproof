//! Unary file admission, token wait and bounded identity JSON body decoding.
//! This is a component of the future RPC handler, not protocol negotiation or
//! Basic account middleware. Linux `FilePool::prepare_rpc` supplies the explicit
//! account preflight; the caller must still select the supported route/codec.
use crate::{
    AuthPermit, MetadataSource, ServiceError, Session,
    file_upload::{self, HeaderLimits},
    owned_body::Owner,
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
    sync::{OwnedSemaphorePermit, Semaphore},
    time::{Instant, timeout_at},
};

mod timeout;
pub use timeout::{InvalidTimeout, TimeoutReason};
#[cfg(test)]
pub(crate) mod tests;

/// No production defaults. Capacity includes retained headers, accepted frames,
/// collected JSON, decoding work, decoded records and downstream response work.
/// Socket/header parsing and allocations before this boundary are separate.
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

    /// Decode a selected unary file method after protocol/Basic middleware.
    /// The trusted HTTP framing layer supplies the unconsumed Body; an exact
    /// size hint is its length, never a reparse of Content-Length. Applies the
    /// fixed Connect timeout now, capped by the admission's original deadline.
    /// # Errors
    /// Invalid timeout, body length/frame/trailer/transport failures, malformed
    /// JSON, unknown method, expiry or lost decoder. No file effects occur here.
    pub async fn decode(self, method: &str, body: Body) -> Result<Decoded, Failure> {
        let mut deadline = self.admitted.deadline;
        let owner: Owner = self.admitted.lease.clone();
        let result = async {
            check_deadline(deadline)?;
            deadline = timeout::deadline(&self.headers, Instant::now(), deadline)
                .map_err(Error::Timeout)?;
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
            let task = tokio::task::spawn_blocking(move || {
                check_deadline(deadline)?;
                let request = file_protocol::decode_request(method, &payload, limit)
                    .map_err(Error::Protocol)?;
                drop(payload);
                check_deadline(deadline)?;
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
    Timeout(InvalidTimeout),
    BodyLimit,
    Transport,
    Protocol(ProtocolError),
    Unavailable,
}
/// Classified only. No invented RPC status/text and no automatic replay.
pub struct Failure {
    error: Error,
    deadline: Instant,
    _owner: Owner,
}
impl Failure {
    #[must_use]
    pub const fn error(&self) -> &Error {
        &self.error
    }
    #[must_use]
    pub const fn deadline(&self) -> Instant {
        self.deadline
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
