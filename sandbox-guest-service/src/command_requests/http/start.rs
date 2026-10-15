//! Start failures retain the original account/worker observation and request.
use super::{ResponseFormat, default_account, reply};
use crate::{command_requests::Error as RequestError, owned_body::Owner};
use axum::response::Response;
use std::{fmt, sync::Arc};
use tokio::time::Instant;

mod native;
#[cfg(test)]
mod tests;
pub(crate) use native::NativeFailure;

#[derive(Debug)]
pub enum Failure {
    Account(default_account::Failure),
    Observed(ObservedFailure),
}
impl Failure {
    #[must_use]
    pub const fn error(&self) -> reply::Error {
        match self {
            Self::Account(f) => f.error(),
            Self::Observed(f) => f.error,
        }
    }
    #[must_use]
    pub fn request_error(&self) -> RequestError {
        match self {
            Self::Account(f) => f.request_error(),
            Self::Observed(f) => f.source.request_error.clone(),
        }
    }
    #[must_use]
    pub fn deadline(&self) -> Instant {
        match self {
            Self::Account(f) => f.deadline(),
            Self::Observed(f) => f.source.deadline,
        }
    }
}

struct Source {
    missing_cwd: Option<String>,
    native_start: Option<NativeFailure>,
    format: Option<ResponseFormat>,
    request_error: RequestError,
    deadline: Instant,
    _owner: Owner,
}
pub struct ObservedFailure {
    error: reply::Error,
    source: Arc<Source>,
}
impl fmt::Debug for ObservedFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CommandStartReplyFailure")
            .field("error", &self.error)
            .finish_non_exhaustive()
    }
}

#[cfg(target_os = "linux")]
impl crate::command_requests::Failure {
    /// Render an observed Start failure, without a second account or path
    /// lookup. The legacy account sender remains available independently.
    /// # Errors
    /// Unclassified failure, invalid budgets, expiry or failed encoding. The
    /// original request and actual preparation cleanup keep their owners.
    pub async fn into_start_response(mut self, limits: reply::Limits) -> Result<Response, Failure> {
        if self.missing_cwd.is_none() && self.native_start.is_none() {
            return self
                .into_default_account_response(limits)
                .await
                .map_err(Failure::Account);
        }
        let missing_cwd = if self.error()
            == RequestError::Registration(crate::command::RegistrationError::Preparation(
                crate::command::CommandError::InvalidArgument,
            )) {
            self.missing_cwd.take()
        } else {
            None
        };
        let native_start = self.native_start.take().filter(|native| {
            native.errno > 0
                && self.error()
                    == RequestError::Command(crate::command::CommandError::ExecFailed(native.errno))
        });
        let source = Source {
            missing_cwd,
            native_start,
            format: self.response,
            request_error: self.error(),
            deadline: self.deadline(),
            _owner: Arc::new(self),
        };
        send(source, limits).await.map_err(Failure::Observed)
    }
}

async fn send(source: Source, limits: reply::Limits) -> Result<Response, ObservedFailure> {
    let source = Arc::new(source);
    let result = async {
        let format = source.format.ok_or(reply::Error::UnclassifiedFault)?;
        if source.missing_cwd.is_some() == source.native_start.is_some() {
            return Err(reply::Error::UnclassifiedFault);
        }
        let worker = source.clone();
        reply::send_stream_error(
            format,
            limits,
            source.deadline,
            source.clone(),
            move |limit| match (&worker.missing_cwd, &worker.native_start) {
                (Some(path), None) => payload(path, limit),
                (None, Some(native)) => native.payload(limit),
                _ => Err(reply::Error::UnclassifiedFault),
            },
        )
        .await
    }
    .await;
    result.map_err(|error| ObservedFailure { error, source })
}

fn payload(path: &str, limit: usize) -> Result<Vec<u8>, reply::Error> {
    crate::go_json::bounded(limit as u64, |put| {
        put(br#"{"error":{"code":"invalid_argument","message":"cwd '"#)?;
        crate::go_json::string_content(path.as_bytes(), put)?;
        put(br#"' does not exist"}}"#)
    })
    .map_err(|_| reply::Error::ResponseLimit)
}
