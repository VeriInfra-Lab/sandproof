//! Post-header file request admission, before query parsing or authorization.
//! This is not socket/header-parser admission or an installed HTTP listener.
use crate::{
    MetadataSource, Session, file_authorization as auth, file_query, file_upload, upload_response,
};
use axum::{
    http::{HeaderMap, Method},
    response::Response,
};
use std::{sync::Arc, time::Duration};
use tokio::{
    sync::{OwnedSemaphorePermit, Semaphore},
    time::Instant,
};

#[cfg(test)]
pub(crate) mod tests;

/// Explicit component budgets, without production defaults. A slot covers
/// admitted parsing/auth waits, retained capabilities, rejections and actual
/// transfer work/results. Connections and already parsed HTTP buffers remain
/// separately bounded by the listener/transport.
#[derive(Clone, Copy, Debug)]
pub struct Limits {
    pub in_flight: usize,
    pub timeout: Duration,
    pub query: file_query::QueryLimits,
    pub authorization: auth::Limits,
    pub rejection: upload_response::Limits,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AdmissionError {
    InvalidConfiguration,
    ResourceExhausted,
}

/// A clone shares the same finite pool. Admission has no wait queue and does
/// not spawn rejection work for unadmitted requests. The listener must reject
/// those requests within its own separately bounded response/connection budget.
#[derive(Clone)]
pub struct FileRequests {
    slots: Arc<Semaphore>,
    limits: Limits,
}
impl FileRequests {
    /// # Errors
    /// Zero, invalid or unrepresentable component budgets.
    pub fn new(limits: Limits) -> Result<Self, AdmissionError> {
        if limits.in_flight == 0
            || limits.in_flight > Semaphore::MAX_PERMITS
            || limits.timeout.is_zero()
            || Instant::now().checked_add(limits.timeout).is_none()
            || file_query::parse(b"", limits.query).is_err()
            || file_upload::check_headers(&HeaderMap::new(), limits.authorization.headers).is_err()
            || limits.authorization.query_bytes == 0
            || limits.authorization.query_bytes > isize::MAX as usize / 16
            || limits.rejection.validate().is_err()
        {
            return Err(AdmissionError::InvalidConfiguration);
        }
        Ok(Self {
            slots: Arc::new(Semaphore::new(limits.in_flight)),
            limits,
        })
    }

    /// Call before parsing query fields or waiting for session authorization.
    /// The absolute deadline begins here and is never restarted by the file pool.
    /// # Errors
    /// Pool saturation or a deadline that can no longer be represented.
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

#[derive(Debug)]
struct Lease {
    _slot: OwnedSemaphorePermit,
}

/// Non-cloneable admission. Dropping it cancels preparation. Successful
/// authorization moves its slot into the opaque file capability; downstream
/// account/file workers and response bytes retain that same original slot.
pub struct Admitted {
    lease: Arc<Lease>,
    limits: Limits,
    deadline: Instant,
}
impl Admitted {
    #[must_use]
    pub const fn deadline(&self) -> Instant {
        self.deadline
    }

    /// Check the complete already parsed header map, then bind the original
    /// query before authorization. No body read, account lookup or file effect.
    /// # Errors
    /// Expiry, unsupported method, header/query budget or original binding/auth
    /// rejection. An error retains admission until dropped or fully delivered.
    pub async fn authorize<M: MetadataSource>(
        self,
        session: &Session<M>,
        method: &Method,
        headers: &HeaderMap,
        raw_query: &[u8],
    ) -> Result<auth::Authorization, Rejected> {
        let result = self.prepare(session, method, headers, raw_query).await;
        match result {
            Ok(mut authorization) => {
                authorization.request_owner = Some(self.lease);
                Ok(authorization)
            }
            Err(error) => Err(Rejected {
                error,
                lease: self.lease,
                deadline: self.deadline,
                limits: self.limits.rejection,
            }),
        }
    }

    async fn prepare<M: MetadataSource>(
        &self,
        session: &Session<M>,
        method: &Method,
        headers: &HeaderMap,
        raw_query: &[u8],
    ) -> Result<auth::Authorization, Error> {
        if Instant::now() >= self.deadline {
            return Err(Error::DeadlineExceeded);
        }
        let operation = match *method {
            Method::GET | Method::HEAD => auth::Operation::Read,
            Method::POST => auth::Operation::Write,
            _ => return Err(Error::UnsupportedMethod),
        };
        file_upload::check_headers(headers, self.limits.authorization.headers)
            .map_err(|_| Error::HeaderLimit)?;
        let query = file_query::parse(raw_query, self.limits.query).map_err(Error::Query)?;
        session
            .authorize_file(
                headers,
                query,
                operation,
                self.limits.authorization,
                self.deadline,
            )
            .await
            .map_err(Error::Authorization)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Error {
    DeadlineExceeded,
    UnsupportedMethod,
    HeaderLimit,
    Query(file_query::BindingError),
    Authorization(auth::Error),
}

/// Retains the original request slot, even while awaiting error delivery.
/// Diagnostics never contain a credential; a binding error may retain its
/// bounded original expiration bytes for the fixed Go error message.
#[derive(Debug)]
pub struct Rejected {
    error: Error,
    lease: Arc<Lease>,
    deadline: Instant,
    limits: upload_response::Limits,
}
#[derive(Debug, PartialEq, Eq)]
pub enum ResponseError {
    Unmapped(Error),
    Delivery(upload_response::Error),
}
impl Rejected {
    #[must_use]
    pub const fn error(&self) -> &Error {
        &self.error
    }

    /// Render only frozen query binding and authentication denials. Other
    /// failures remain classified for the future listener's failure policy;
    /// this layer does not invent wire behavior for budgets or clock failures.
    /// # Errors
    /// Unmapped failure or failed/expired/over-budget response generation.
    pub async fn into_response(self) -> Result<Response, ResponseError> {
        let response = match self.error {
            Error::Query(
                error @ (file_query::BindingError::Duplicate(_)
                | file_query::BindingError::Integer { .. }),
            ) => file_query::rejected(error, self.limits, self.deadline, self.lease).await,
            Error::Authorization(auth::Error::Denied(denial)) => {
                auth::rejected(denial, self.limits, self.deadline, self.lease).await
            }
            error => return Err(ResponseError::Unmapped(error)),
        };
        response.map_err(ResponseError::Delivery)
    }
}
