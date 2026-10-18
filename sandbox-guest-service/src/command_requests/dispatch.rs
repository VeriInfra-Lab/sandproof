//! PID/tag dispatch under the original request. Known selector/parameter
//! failures preserve their actual observation for the separate HTTP sender.
use super::http::operation::{ChannelRejection, Rejection};
use super::{Body, Error, Failure, Instant, Owner, Ready, Request, check_deadline};
use crate::command::{CommandBodyLimits, CommandError, InputFailure, ManagedCommand, PidRegistry};
use sandproof_guest_runtime::{
    ProcessSignal, PtySize,
    protocol::{self, Input, Response, Selector, Signal},
};
use tokio::time::timeout_at;

pub struct Reply {
    body: Body,
    stream: bool,
    deadline: Instant,
    format: Option<super::http::ResponseFormat>,
    owner: Owner,
}
impl Reply {
    pub(super) fn streaming(
        body: Body,
        deadline: Instant,
        format: Option<super::http::ResponseFormat>,
        owner: Owner,
    ) -> Self {
        Self {
            body,
            stream: true,
            deadline,
            format,
            owner,
        }
    }
    #[must_use]
    pub const fn is_stream(&self) -> bool {
        self.stream
    }
    #[must_use]
    pub const fn deadline(&self) -> Instant {
        self.deadline
    }
    /// Low-level identity bytes without HTTP formatting. Capacity follows every
    /// returned Bytes clone. Selected HTTP callers use `into_http_response`.
    pub fn into_body(self) -> Body {
        self.body
    }

    /// Apply this request's original response format to its completed operation
    /// or original command subscription. No new admission or operation occurs.
    /// # Errors
    /// Missing selection, invalid limits, encoding or expiry. A response failure
    /// never authorizes replaying the operation or restarting the command.
    pub async fn into_http_response(
        self,
        limits: super::http::response::Limits,
    ) -> Result<axum::response::Response, super::http::response::Failure> {
        super::http::response::send(
            self.body,
            self.stream,
            self.format,
            limits,
            self.deadline,
            self.owner,
        )
        .await
    }
}
impl Ready {
    /// Consume one authenticated, decoded request and dispatch to the original
    /// PID or exact tag capability. Each request repeats token/Basic preflight.
    /// Start uses the separate `start_pid` entry. A successful control reply is not an exit
    /// receipt. Cancellation of a write may follow a delivered prefix.
    /// # Errors
    /// Expiry, unsupported operation, missing/unknown PID, I/O or response budget.
    /// No automatic retry, PID adoption, execution-deadline reset or stdin close.
    pub async fn execute_pid(
        self,
        registry: &PidRegistry,
        limits: CommandBodyLimits,
    ) -> Result<Reply, Failure> {
        let limits = self.response_limits(limits);
        let deadline = self.deadline();
        let format = self.response;
        let owner = self.decoded.authenticated.admitted.owner.clone();
        let mut observed = None;
        let result = async {
            check_deadline(deadline)?;
            let request = &self.decoded.request;
            if let Request::Connect(selector) = request {
                let command = select(registry, selector.as_ref(), true, &mut observed)?;
                let stream = command
                    .subscribe(command.identity())
                    .map_err(Error::Command)?;
                let body = stream
                    .into_body(owner.clone(), limits, deadline)
                    .map_err(Error::Response)?;
                check_deadline(deadline)?;
                return Ok(Reply::streaming(body, deadline, format, owner.clone()));
            }
            let bytes = match request {
                Request::List => registry
                    .list_json(limits.message_bytes)
                    .map_err(Error::Registry)?,
                Request::Start(_) | Request::StartArgumentsExceeded { .. } => {
                    return Err(Error::UnsupportedOperation);
                }
                _ => {
                    let bytes = protocol::encode_response(Response::Empty, limits.message_bytes)
                        .map_err(Error::Protocol)?;
                    control(request, registry, &mut observed).await?;
                    bytes
                }
            };
            check_deadline(deadline)?;
            let chunk = Unary {
                bytes,
                _owner: owner.clone(),
            };
            Ok(Reply {
                body: Body::from_stream(futures_util::stream::once(async move {
                    check_deadline(deadline)?;
                    Ok::<_, Error>(bytes::Bytes::from_owner(chunk))
                })),
                stream: false,
                deadline,
                format,
                owner: owner.clone(),
            })
        };
        timeout_at(deadline, result)
            .await
            .unwrap_or(Err(Error::DeadlineExceeded))
            .map_err(|error| {
                let observed = observed.filter(|observation| observation.matches(&error));
                let mut failure = Failure::new(error, deadline, owner);
                failure.response = format;
                failure.operation = observed;
                failure
            })
    }
}
fn select(
    registry: &PidRegistry,
    selector: Option<&Selector>,
    retained: bool,
    observed: &mut Option<Rejection>,
) -> Result<ManagedCommand, Error> {
    let result = match selector {
        Some(Selector::Pid(pid)) => {
            if retained {
                registry.connectable(*pid)
            } else {
                registry.live(*pid)
            }
        }
        Some(Selector::Tag(tag)) => {
            if retained {
                registry.connectable_tag(tag)
            } else {
                registry.live_tag(tag)
            }
        }
        _ => {
            *observed = Some(Rejection::MissingSelector);
            return Err(Error::Protocol(protocol::ProtocolError::Unimplemented));
        }
    };
    result.map_err(|error| {
        if error == crate::command::RegistryError::NotFound {
            *observed = match selector {
                Some(Selector::Pid(pid)) => Some(Rejection::PidNotFound(*pid)),
                Some(Selector::Tag(tag)) => Some(Rejection::TagNotFound(tag.clone())),
                _ => None,
            };
        }
        Error::Registry(error)
    })
}
fn live(
    registry: &PidRegistry,
    selector: Option<&Selector>,
    observed: &mut Option<Rejection>,
) -> Result<ManagedCommand, Error> {
    select(registry, selector, false, observed)
}
async fn control(
    request: &Request,
    registry: &PidRegistry,
    observed: &mut Option<Rejection>,
) -> Result<(), Error> {
    let result = match request {
        Request::SendInput { process, input } => {
            let command = live(registry, process.as_ref(), observed)?;
            match input {
                Some(Input::Stdin(bytes)) => {
                    let result = command
                        .write_input_observed(command.identity(), bytes)
                        .await;
                    observe_input(&result, command.identity().pid, false, observed);
                    channel_result(
                        result.map_err(InputFailure::error),
                        command.has_pty(),
                        ChannelRejection::StdinOnPty,
                        observed,
                    )
                }
                Some(Input::Pty(bytes)) => {
                    let result = command.write_pty_observed(command.identity(), bytes).await;
                    observe_input(&result, command.identity().pid, true, observed);
                    channel_result(
                        result.map_err(InputFailure::error),
                        !command.has_pty(),
                        ChannelRejection::PtyOnPipe,
                        observed,
                    )
                }
                _ => {
                    *observed = Some(Rejection::MissingInput);
                    return Err(Error::Protocol(protocol::ProtocolError::Unimplemented));
                }
            }
        }
        Request::CloseStdin(process) => {
            let command = live(registry, process.as_ref(), observed)?;
            channel_result(
                command.close_input(command.identity()),
                command.has_pty(),
                ChannelRejection::ClosePty,
                observed,
            )
        }
        Request::SendSignal { process, signal } => {
            let command = live(registry, process.as_ref(), observed)?;
            let signal = match signal {
                Signal::Terminate => ProcessSignal::Terminate,
                Signal::Kill => ProcessSignal::Kill,
                signal => {
                    *observed = Some(Rejection::Signal(match signal {
                        Signal::Unknown(value) => *value,
                        _ => 0,
                    }));
                    return Err(Error::Protocol(protocol::ProtocolError::Unimplemented));
                }
            };
            command
                .signal_observed(command.identity(), signal)
                .map_err(|error| {
                    if let crate::command::SignalFailure::Native(errno) = &error {
                        *observed = Some(Rejection::NativeSignal(*errno));
                    }
                    error.error()
                })
        }
        Request::Update { process, pty } => {
            let command = live(registry, process.as_ref(), observed)?;
            if let Some(pty) = pty {
                let size = pty.size.as_ref().map_or_else(PtySize::default, |size| {
                    PtySize::from_wire(size.cols, size.rows)
                });
                let result = command
                    .resize_pty_observed(command.identity(), size)
                    .map_err(|error| {
                        if let crate::command::ResizeFailure::Native { errno, .. } = &error {
                            *observed = Some(Rejection::NativeResize(*errno));
                        }
                        error.error()
                    });
                channel_result(
                    result,
                    !command.has_pty(),
                    ChannelRejection::ResizePipe,
                    observed,
                )
            } else {
                Ok(())
            }
        }
        _ => return Err(Error::UnsupportedOperation),
    };
    result.map_err(Error::Command)
}

fn observe_input(
    result: &Result<(), InputFailure>,
    pid: u32,
    pty: bool,
    observed: &mut Option<Rejection>,
) {
    *observed = match result {
        Err(InputFailure::StdinClosed) => Some(Rejection::StdinClosed),
        Err(InputFailure::StdinDisabled) => Some(Rejection::StdinDisabled),
        Err(InputFailure::Native { errno, .. }) => Some(Rejection::NativeInput {
            pid,
            pty,
            errno: *errno,
        }),
        _ => None,
    };
}
// The original operation must actually reject, and its immutable terminal
// capability must establish the precise reason. A live PTY ioctl EINVAL,
// completion race, busy writer, closed input or I/O failure is not this fact.
fn channel_result(
    result: Result<(), CommandError>,
    wrong_mode: bool,
    rejection: ChannelRejection,
    observed: &mut Option<Rejection>,
) -> Result<(), CommandError> {
    if observed.is_none() && wrong_mode && result == Err(CommandError::InvalidArgument) {
        *observed = Some(Rejection::Channel(rejection));
    }
    result
}

struct Unary {
    bytes: Vec<u8>,
    _owner: Owner,
}
impl AsRef<[u8]> for Unary {
    fn as_ref(&self) -> &[u8] {
        &self.bytes
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn input_observation_keeps_original_pid_errno_and_never_relabels_other_failures() {
        for (errno, error) in [
            (32, CommandError::Unavailable),
            (22, CommandError::InvalidArgument),
        ] {
            let mut observed = None;
            observe_input(
                &Err(InputFailure::Native { error, errno }),
                123,
                true,
                &mut observed,
            );
            assert_eq!(
                channel_result(Err(error), true, ChannelRejection::PtyOnPipe, &mut observed),
                Err(error)
            );
            assert!(
                matches!(observed, Some(Rejection::NativeInput { pid:123, pty:true, errno: number }) if number == errno)
            );
            let fact = observed.unwrap();
            assert!(fact.matches(&Error::Command(error)));
            assert!(!fact.matches(&Error::DeadlineExceeded));
            assert!(!fact.matches(&Error::Command(CommandError::ClosedInput)));
        }
        for result in [
            Ok(()),
            Err(InputFailure::Command(CommandError::Unavailable)),
            Err(InputFailure::Command(CommandError::Cancelled)),
            Err(InputFailure::Command(CommandError::ResourceExhausted)),
        ] {
            let mut observed = None;
            observe_input(&result, 123, false, &mut observed);
            assert!(observed.is_none());
        }
        assert!(
            !Rejection::NativeInput {
                pid: 123,
                pty: false,
                errno: 0
            }
            .matches(&Error::Command(CommandError::Unavailable))
        );
    }

    #[test]
    fn channel_observation_requires_original_mode_and_actual_rejection() {
        // EINVAL from the original ioctl is already explained, even though
        // the legacy coarse error is identical to a pipe-mode rejection.
        let mut observed = Some(Rejection::NativeResize(22));
        assert_eq!(
            channel_result(
                Err(CommandError::InvalidArgument),
                true,
                ChannelRejection::ResizePipe,
                &mut observed,
            ),
            Err(CommandError::InvalidArgument)
        );
        assert!(matches!(observed, Some(Rejection::NativeResize(22))));
        let outcomes = [
            Ok(()),
            Err(CommandError::InvalidArgument),
            Err(CommandError::Cancelled),
            Err(CommandError::StaleIdentity),
            Err(CommandError::ResourceExhausted),
            Err(CommandError::NotStarted),
            Err(CommandError::ClosedInput),
            Err(CommandError::DeadlineExceeded),
            Err(CommandError::LaunchUnknown),
            Err(CommandError::ExecFailed(22)),
            Err(CommandError::Lagged),
            Err(CommandError::Unavailable),
        ];
        for wrong_mode in [false, true] {
            for result in outcomes {
                let mut observed = None;
                assert_eq!(
                    channel_result(
                        result,
                        wrong_mode,
                        ChannelRejection::ResizePipe,
                        &mut observed
                    ),
                    result
                );
                assert_eq!(
                    matches!(
                        observed,
                        Some(Rejection::Channel(ChannelRejection::ResizePipe))
                    ),
                    wrong_mode && result == Err(CommandError::InvalidArgument)
                );
                if let Some(observed) = observed {
                    assert!(observed.matches(&Error::Command(CommandError::InvalidArgument)));
                    assert!(!observed.matches(&Error::Command(CommandError::Unavailable)));
                    assert!(
                        !observed.matches(&Error::Protocol(protocol::ProtocolError::Unimplemented))
                    );
                }
            }
        }
    }
}
