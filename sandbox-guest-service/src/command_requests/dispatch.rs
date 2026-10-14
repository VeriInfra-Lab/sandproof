//! PID-only dispatch under the original request. Errors deliberately remain
//! payload-free component errors; public Connect status/text mapping is separate.
use super::{Body, Error, Failure, Instant, Owner, Ready, Request, check_deadline};
use crate::command::{CommandBodyLimits, ManagedCommand, PidRegistry};
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
    /// PID capability only. Each later request must repeat token/Basic preflight.
    /// Start uses the separate `start_pid` entry; tag selectors remain unsupported
    /// pending their policy/assembly. A successful control reply is not an exit
    /// receipt. Cancellation of a write may follow a delivered prefix.
    /// # Errors
    /// Expiry, unsupported operation, missing/unknown PID, I/O or response budget.
    /// No automatic retry, PID adoption, execution-deadline reset or stdin close.
    pub async fn execute_pid(
        self,
        registry: &PidRegistry,
        limits: CommandBodyLimits,
    ) -> Result<Reply, Failure> {
        let deadline = self.deadline();
        let format = self.response;
        let owner = self.decoded.authenticated.admitted.owner.clone();
        let result = async {
            check_deadline(deadline)?;
            let request = &self.decoded.request;
            if let Request::Connect(selector) = request {
                let command = registry
                    .connectable(pid(selector.as_ref())?)
                    .map_err(Error::Registry)?;
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
                Request::Start(_) => return Err(Error::UnsupportedOperation),
                _ => {
                    let bytes = protocol::encode_response(Response::Empty, limits.message_bytes)
                        .map_err(Error::Protocol)?;
                    control(request, registry).await?;
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
                let mut failure = Failure::new(error, deadline, owner);
                failure.response = format;
                failure
            })
    }
}
fn pid(selector: Option<&Selector>) -> Result<u32, Error> {
    match selector {
        Some(Selector::Pid(pid)) => Ok(*pid),
        Some(Selector::Tag(_)) => Err(Error::UnsupportedOperation),
        _ => Err(Error::Protocol(protocol::ProtocolError::Unimplemented)),
    }
}
fn live(registry: &PidRegistry, selector: Option<&Selector>) -> Result<ManagedCommand, Error> {
    registry.live(pid(selector)?).map_err(Error::Registry)
}
async fn control(request: &Request, registry: &PidRegistry) -> Result<(), Error> {
    let result = match request {
        Request::SendInput { process, input } => {
            let command = live(registry, process.as_ref())?;
            match input {
                Some(Input::Stdin(bytes)) => command.write_input(command.identity(), bytes).await,
                Some(Input::Pty(bytes)) => command.write_pty(command.identity(), bytes).await,
                _ => return Err(Error::Protocol(protocol::ProtocolError::Unimplemented)),
            }
        }
        Request::CloseStdin(process) => {
            let command = live(registry, process.as_ref())?;
            command.close_input(command.identity())
        }
        Request::SendSignal { process, signal } => {
            let command = live(registry, process.as_ref())?;
            let signal = match signal {
                Signal::Terminate => ProcessSignal::Terminate,
                Signal::Kill => ProcessSignal::Kill,
                _ => return Err(Error::Protocol(protocol::ProtocolError::Unimplemented)),
            };
            command.signal(command.identity(), signal)
        }
        Request::Update { process, pty } => {
            let command = live(registry, process.as_ref())?;
            if let Some(pty) = pty {
                let size = pty.size.as_ref().map_or_else(PtySize::default, |size| {
                    PtySize::from_wire(size.cols, size.rows)
                });
                command.resize_pty(command.identity(), size)
            } else {
                Ok(())
            }
        }
        _ => return Err(Error::UnsupportedOperation),
    };
    result.map_err(Error::Command)
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
