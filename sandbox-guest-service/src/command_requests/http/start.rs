//! Start failures retain the original account/worker observation and request.
use super::{ResponseFormat, default_account, reply};
use crate::{command_requests::Error as RequestError, owned_body::Owner};
use axum::response::Response;
use std::{fmt, sync::Arc};
use tokio::time::Instant;

mod native;
#[cfg(test)]
mod tests;
pub(crate) use native::{LaunchFailure, LaunchFailureKind, PipeStream, PtyStage};

// Exactly one observation from this Start, never reconstructed from its coarse
// error or from a later path/account lookup.
pub(crate) enum Rejection {
    TagAlreadyExists(String),
    MissingCwd(String),
    Launch(LaunchFailure),
    Expansion(crate::upload_response::PathExpansion),
}
impl Rejection {
    #[cfg(target_os = "linux")]
    fn matches(&self, error: &RequestError) -> bool {
        use crate::command::{CommandError, RegistrationError};
        match self {
            Self::TagAlreadyExists(_) => {
                *error == RequestError::Registry(crate::command::RegistryError::AlreadyExists)
            }
            Self::MissingCwd(_) => {
                *error
                    == RequestError::Registration(RegistrationError::Preparation(
                        CommandError::InvalidArgument,
                    ))
            }
            Self::Launch(launch) => match launch.kind {
                LaunchFailureKind::Native(errno) => {
                    errno > 0 && *error == RequestError::Command(CommandError::ExecFailed(errno))
                }
                LaunchFailureKind::Pipe { errno, .. } => {
                    errno > 0
                        && launch.is_pipe()
                        && *error
                            == RequestError::Registration(RegistrationError::Preparation(
                                crate::command::prepare_error(&std::io::Error::from_raw_os_error(
                                    errno,
                                )),
                            ))
                }
                LaunchFailureKind::Pty { errno, .. } => {
                    errno > 0
                        && !launch.is_pipe()
                        && *error
                            == RequestError::Registration(RegistrationError::Preparation(
                                crate::command::prepare_error(&std::io::Error::from_raw_os_error(
                                    errno,
                                )),
                            ))
                }
                LaunchFailureKind::EnvironmentNul | LaunchFailureKind::ArgumentNul => {
                    *error
                        == RequestError::Registration(RegistrationError::Preparation(
                            CommandError::InvalidArgument,
                        ))
                }
            },
            Self::Expansion(_) => {
                *error == RequestError::Context(crate::ContextError::InvalidArgument)
            }
        }
    }

    fn payload(&self, limit: usize) -> Result<Vec<u8>, reply::Error> {
        match self {
            Self::TagAlreadyExists(tag) => message_with_code(
                b"already_exists",
                &[b"process with tag ", tag.as_bytes(), b" already exists"],
                limit,
            ),
            Self::MissingCwd(path) => {
                message(&[b"cwd '", path.as_bytes(), b"' does not exist"], limit)
            }
            Self::Launch(native) => native.payload(limit),
            Self::Expansion(expansion) => message(&expansion.rpc_parts(), limit),
        }
    }
}

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
    rejection: Option<Rejection>,
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
        if self.start_rejection.is_none() {
            return self
                .into_default_account_response(limits)
                .await
                .map_err(Failure::Account);
        }
        let rejection = self
            .start_rejection
            .take()
            .filter(|fact| fact.matches(&self.error()));
        let source = Source {
            rejection,
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
        if source.rejection.is_none() {
            return Err(reply::Error::UnclassifiedFault);
        }
        let worker = source.clone();
        reply::send_stream_error(
            format,
            limits,
            source.deadline,
            source.clone(),
            move |limit| {
                worker
                    .rejection
                    .as_ref()
                    .ok_or(reply::Error::UnclassifiedFault)?
                    .payload(limit)
            },
        )
        .await
    }
    .await;
    result.map_err(|error| ObservedFailure { error, source })
}

fn message(parts: &[&[u8]], limit: usize) -> Result<Vec<u8>, reply::Error> {
    message_with_code(b"invalid_argument", parts, limit)
}

fn message_with_code(code: &[u8], parts: &[&[u8]], limit: usize) -> Result<Vec<u8>, reply::Error> {
    crate::go_json::bounded(limit as u64, |put| {
        put(br#"{"error":{"code":""#)?;
        put(code)?;
        put(br#"","message":""#)?;
        for part in parts {
            crate::go_json::string_content(part, put)?;
        }
        put(br#""}}"#)
    })
    .map_err(|_| reply::Error::ResponseLimit)
}
