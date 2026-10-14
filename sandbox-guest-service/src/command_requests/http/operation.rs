//! Fixed PID lookup and parameter rejections observed during original dispatch.
//! No tag policy or arbitrary worker/I/O failure is converted into public text.
use super::{ResponseFormat, reply};
use crate::{command_requests::Error as RequestError, owned_body::Owner};
use axum::{http::StatusCode, response::Response};
use std::{fmt, sync::Arc};
use tokio::time::Instant;

#[cfg(test)]
mod tests;

#[derive(Clone, Copy)]
pub(crate) enum Rejection {
    MissingSelector,
    PidNotFound(u32),
    MissingInput,
    Signal(i32),
}
impl Rejection {
    #[cfg(target_os = "linux")]
    pub(crate) fn matches(self, error: RequestError) -> bool {
        match self {
            Self::PidNotFound(_) => {
                error == RequestError::Registry(crate::command::RegistryError::NotFound)
            }
            _ => {
                error
                    == RequestError::Protocol(
                        sandproof_guest_runtime::protocol::ProtocolError::Unimplemented,
                    )
            }
        }
    }
}
struct Source {
    rejection: Option<Rejection>,
    format: Option<ResponseFormat>,
    request_error: RequestError,
    deadline: Instant,
    _owner: Owner,
}
pub struct Failure {
    error: reply::Error,
    source: Arc<Source>,
}
impl Failure {
    #[must_use]
    pub const fn error(&self) -> reply::Error {
        self.error
    }
    #[must_use]
    pub fn request_error(&self) -> RequestError {
        self.source.request_error
    }
    #[must_use]
    pub fn deadline(&self) -> Instant {
        self.source.deadline
    }
}
impl fmt::Debug for Failure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CommandOperationReplyFailure")
            .field("error", &self.error)
            .finish_non_exhaustive()
    }
}
#[cfg(target_os = "linux")]
impl crate::command_requests::Failure {
    /// Emit only a lookup/parameter failure observed by the original PID dispatch.
    /// No new lookup, operation, token check or deadline is created here.
    /// # Errors
    /// Missing observation/format, invalid limits, expiry or encoding failure.
    /// The original error and request responsibility survive every refusal.
    pub async fn into_operation_response(self, limits: reply::Limits) -> Result<Response, Failure> {
        let source = Source {
            rejection: self.operation.filter(|r| r.matches(self.error())),
            format: self.response,
            request_error: self.error(),
            deadline: self.deadline(),
            _owner: Arc::new(self),
        };
        send(source, limits).await
    }
}
async fn send(source: Source, limits: reply::Limits) -> Result<Response, Failure> {
    let source = Arc::new(source);
    let result = async {
        let format = source.format.ok_or(reply::Error::UnclassifiedFault)?;
        let rejection = source.rejection.ok_or(reply::Error::UnclassifiedFault)?;
        let status = if matches!(rejection, Rejection::PidNotFound(_)) {
            StatusCode::NOT_FOUND
        } else {
            StatusCode::NOT_IMPLEMENTED
        };
        reply::send_error(
            format,
            status,
            limits,
            source.deadline,
            source.clone(),
            move |stream, limit| payload(rejection, stream, limit),
        )
        .await
    }
    .await;
    result.map_err(|error| Failure { error, source })
}
fn payload(rejection: Rejection, stream: bool, limit: usize) -> Result<Vec<u8>, reply::Error> {
    crate::go_json::bounded(limit as u64, |put| {
        if stream {
            put(br#"{"error":"#)?;
        }
        put(if matches!(rejection, Rejection::PidNotFound(_)) {
            br#"{"code":"not_found","message":""#
        } else {
            br#"{"code":"unimplemented","message":""#
        })?;
        let mut message = |bytes: &[u8]| crate::go_json::string_content(bytes, put);
        match rejection {
            Rejection::MissingSelector => message(b"invalid input type *process.ProcessSelector")?,
            Rejection::MissingInput => message(b"invalid input type <nil>")?,
            Rejection::PidNotFound(pid) => {
                message(b"process with pid ")?;
                message(pid.to_string().as_bytes())?;
                message(b" not found")?;
            }
            Rejection::Signal(value) => {
                message(b"invalid signal: ")?;
                if value == 0 {
                    message(b"SIGNAL_UNSPECIFIED")?;
                } else {
                    message(value.to_string().as_bytes())?;
                }
            }
        }
        put(if stream { br#""}}"# } else { br#""}"# })
    })
    .map_err(|_| reply::Error::ResponseLimit)
}
