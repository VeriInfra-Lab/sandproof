//! Fixed PID/parameter rejections and native input/signal errno from dispatch.
//! Unobserved worker/I/O failure and tag policy remain unclassified.
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
    Channel(ChannelRejection),
    StdinClosed,
    StdinDisabled,
    NativeSignal(i32),
    NativeInput { pid: u32, pty: bool, errno: i32 },
}
#[derive(Clone, Copy)]
pub(crate) enum ChannelRejection {
    StdinOnPty,
    PtyOnPipe,
    ClosePty,
    ResizePipe,
}
impl ChannelRejection {
    const fn code(self) -> &'static [u8] {
        if matches!(self, Self::ClosePty) {
            b"unknown"
        } else {
            b"internal"
        }
    }
    const fn message(self) -> &'static [u8] {
        match self {
            Self::StdinOnPty => "error writing to stdin: tty assigned to process — input should be written to the pty, not the stdin".as_bytes(),
            Self::PtyOnPipe => "error writing to tty: tty not assigned to process — input should be written to the stdin, not the tty".as_bytes(),
            Self::ClosePty => "error closing stdin: cannot close stdin for PTY process \u{2014} send Ctrl+D (0x04) instead".as_bytes(),
            Self::ResizePipe => b"error resizing tty: tty not assigned to process",
        }
    }
}
impl Rejection {
    #[cfg(target_os = "linux")]
    pub(crate) fn matches(self, error: &RequestError) -> bool {
        match self {
            Self::NativeSignal(errno) => {
                errno > 0
                    && error == &RequestError::Command(crate::command::CommandError::Unavailable)
            }
            Self::NativeInput { errno, .. } => {
                errno > 0
                    && error
                        == &RequestError::Command(crate::command::input_error(
                            &std::io::Error::from_raw_os_error(errno),
                        ))
            }
            Self::StdinDisabled => {
                error == &RequestError::Command(crate::command::CommandError::ClosedInput)
            }
            Self::StdinClosed => {
                error == &RequestError::Command(crate::command::CommandError::Unavailable)
            }
            Self::Channel(_) => {
                error == &RequestError::Command(crate::command::CommandError::InvalidArgument)
            }
            Self::PidNotFound(_) => {
                error == &RequestError::Registry(crate::command::RegistryError::NotFound)
            }
            _ => {
                error
                    == &RequestError::Protocol(
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
        self.source.request_error.clone()
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
    /// Emit only a failure fact observed by the original PID dispatch/operation.
    /// No new lookup, operation, token check or deadline is created here.
    /// # Errors
    /// Missing observation/format, invalid limits, expiry or encoding failure.
    /// The original error and request responsibility survive every refusal.
    pub async fn into_operation_response(self, limits: reply::Limits) -> Result<Response, Failure> {
        let source = Source {
            rejection: self.operation.filter(|r| r.matches(&self.error())),
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
        if matches!(
            rejection,
            Rejection::Channel(_)
                | Rejection::StdinClosed
                | Rejection::StdinDisabled
                | Rejection::NativeInput { .. }
                | Rejection::NativeSignal(_)
        ) && format.content_type().starts_with("application/connect+")
        {
            return Err(reply::Error::UnclassifiedFault);
        }
        let status = match rejection {
            Rejection::PidNotFound(_) => StatusCode::NOT_FOUND,
            Rejection::Channel(_)
            | Rejection::StdinClosed
            | Rejection::StdinDisabled
            | Rejection::NativeInput { .. }
            | Rejection::NativeSignal(_) => StatusCode::INTERNAL_SERVER_ERROR,
            _ => StatusCode::NOT_IMPLEMENTED,
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
        put(br#"{"code":""#)?;
        put(match rejection {
            Rejection::PidNotFound(_) => b"not_found",
            Rejection::Channel(channel) => channel.code(),
            Rejection::StdinClosed
            | Rejection::StdinDisabled
            | Rejection::NativeInput { .. }
            | Rejection::NativeSignal(_) => b"internal",
            _ => b"unimplemented",
        })?;
        put(br#"","message":""#)?;
        let mut message = |bytes: &[u8]| crate::go_json::string_content(bytes, put);
        match rejection {
            Rejection::NativeSignal(errno) => {
                message(b"error sending signal: ")?;
                // Fixed Go os.Process.Signal converts ESRCH before its caller
                // formats the error; no later status/PID lookup is involved.
                if errno == 3 {
                    message(b"os: process already finished")?;
                } else {
                    message(
                        sandproof_guest_runtime::file_download::LinuxErrno(errno)
                            .message()
                            .as_bytes(),
                    )?;
                }
            }
            Rejection::NativeInput {
                pid, pty, errno, ..
            } => {
                let channel: &[u8] = if pty { b"tty" } else { b"stdin" };
                message(b"error writing to ")?;
                message(channel)?;
                message(b": error writing to ")?;
                message(channel)?;
                message(b" of process '")?;
                message(pid.to_string().as_bytes())?;
                message(b"': write ")?;
                message(if pty { b"/dev/ptmx" } else { b"|1" })?;
                message(b": ")?;
                message(
                    sandproof_guest_runtime::file_download::LinuxErrno(errno)
                        .message()
                        .as_bytes(),
                )?;
            }
            Rejection::Channel(channel) => message(channel.message())?,
            Rejection::StdinClosed | Rejection::StdinDisabled => {
                message(b"error writing to stdin: stdin not enabled or closed")?;
            }
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
