//! Shared command HTTP dispatch with reserved control admission. No listener,
//! production defaults, tag policy or catch-all public error mapping.
use crate::{
    AccountPool, AccountSource, ExecutionSettings, MetadataSource, Session,
    command::{AdmissionLimits, CommandBodyLimits, CommandOwners, PidRegistrar, PidRegistry},
    command_requests::{
        self, CommandRequests, StartPreparation,
        http::{self, Rpc},
    },
    owned_body::Owner,
};
use axum::{body::Body, http::Request, response::Response};
use sandproof_guest_runtime::{ProcessLimits, pid_claims::Binding, protocol};
use std::{
    fmt,
    os::unix::ffi::OsStrExt,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};
use tokio::time::Instant;

/// Independent admission pools. Long streams and potentially blocked input
/// cannot consume the slots reserved for `CloseStdin`, `SendSignal` and Update.
/// Other shared resources (accounts, runtime and sockets) remain separate
/// limits; reserved request slots alone do not guarantee control latency.
pub struct RequestPools {
    pub ordinary: CommandRequests,
    pub control: CommandRequests,
}

/// Trusted resources for one VM/service lifetime. Use one `CommandHttp` assembly
/// (and its clones) as the sole Start dispatcher for this registry/session.
/// Its internal command IDs must not be mixed with IDs from direct preparation.
/// The registrar checks its durable VM binding when claiming the actual PID.
pub struct Processes {
    pub owners: CommandOwners,
    pub registry: PidRegistry,
    pub registrar: PidRegistrar,
    pub binding: Binding,
    pub session: u64,
    pub worker: PathBuf,
    pub private_directory: PathBuf,
    pub process_limits: ProcessLimits,
    pub admission_limits: AdmissionLimits,
}

#[derive(Clone, Copy, Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResponseLimits {
    pub message: CommandBodyLimits,
    pub http: http::reply::Limits,
    pub account_headers: usize,
}
impl ResponseLimits {
    pub(crate) fn validate(self) -> Result<(), InvalidConfiguration> {
        if self.message.validate(Instant::now()).is_err()
            || http::reply::check_limits(self.http).is_err()
            || self.account_headers == 0
            || self.account_headers > isize::MAX as usize / 16
        {
            return Err(InvalidConfiguration);
        }
        Ok(())
    }
}

pub struct CommandHttp<M, S> {
    inner: Arc<Inner<M, S>>,
}
struct Inner<M, S> {
    session: Session<M>,
    requests: RequestPools,
    accounts: AccountPool<S>,
    settings: ExecutionSettings,
    processes: Processes,
    response: ResponseLimits,
    sequence: AtomicU64,
}
impl<M, S> Clone for CommandHttp<M, S> {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
        }
    }
}

/// Preparation either produced an original known rejection, or an immutable
/// decoded request tied to the same authorization, pools, settings and deadline.
pub enum Prepared<M, S> {
    Response(Response),
    Ready(Box<Ready<M, S>>),
}
pub struct Ready<M, S> {
    inner: Arc<Inner<M, S>>,
    operation: command_requests::Ready,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InvalidConfiguration;
impl fmt::Display for InvalidConfiguration {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("invalid command HTTP configuration")
    }
}
impl std::error::Error for InvalidConfiguration {}

impl<M: MetadataSource, S: AccountSource> CommandHttp<M, S> {
    pub(crate) fn shares_session(&self, session: &Session<M>) -> bool {
        self.inner.session.shares_state(session)
    }
    /// Assemble fixed resources without opening sockets or starting processes.
    /// Process/preparation budgets and original registry/store identities are
    /// checked here. The original launch still validates actual paths and I/O;
    /// construction does not prove worker installation or future PID writes.
    /// # Errors
    /// Shared admission pools, mismatched registry/store identity, invalid
    /// worker path syntax, context or budgets. No defaults are selected.
    pub fn new(
        session: Session<M>,
        requests: RequestPools,
        accounts: AccountPool<S>,
        settings: ExecutionSettings,
        processes: Processes,
        response: ResponseLimits,
    ) -> Result<Self, InvalidConfiguration> {
        if requests.ordinary.shares_capacity(&requests.control)
            || !processes
                .registry
                .matches_session(processes.binding, processes.session)
            || !processes.registrar.belongs_to(processes.binding)
            || processes.process_limits.validate().is_err()
            || processes.admission_limits.validate(Instant::now()).is_err()
            || !processes.worker.is_absolute()
            || processes.worker.as_os_str().as_bytes().contains(&0)
            || processes
                .private_directory
                .as_os_str()
                .as_bytes()
                .contains(&0)
            || crate::execution::validate_command_settings(&settings).is_err()
            || response.validate().is_err()
        {
            return Err(InvalidConfiguration);
        }
        Ok(Self {
            inner: Arc::new(Inner {
                session,
                requests,
                accounts,
                settings,
                processes,
                response,
                sequence: AtomicU64::new(0),
            }),
        })
    }

    /// Admit before token/Basic/body work and execute exactly once. Errors from
    /// partial/accepted operations never authorize replay or a replacement PID.
    /// # Errors
    /// Saturation, unknown route, unmapped original faults, expiry or encoding.
    /// Retained failures keep the original capacity until their last owner exits.
    pub async fn handle(&self, request: Request<Body>) -> Result<Response, Failure> {
        match self.prepare(request).await? {
            Prepared::Response(response) => Ok(response),
            Prepared::Ready(ready) => ready.execute().await,
        }
    }

    /// Inspectable preparation for trusted restrictions such as a fixture's
    /// preselected command. Body decoding never starts a command or controls a PID.
    /// # Errors
    /// Same admission/preparation failures as handle, with original ownership.
    pub async fn prepare(&self, request: Request<Body>) -> Result<Prepared<M, S>, Failure> {
        let inner = &self.inner;
        let rpc = route(request.uri().path());
        let pool = if matches!(rpc, Some(Rpc::CloseStdin | Rpc::SendSignal | Rpc::Update)) {
            &inner.requests.control
        } else {
            &inner.requests.ordinary
        };
        let admitted = pool.admit().map_err(Failure::Admission)?;
        let Some(rpc) = rpc else {
            return Err(Failure::Route(Retained {
                deadline: admitted.deadline(),
                _owner: admitted.owner(),
            }));
        };
        let (parts, body) = request.into_parts();
        let authenticated = match admitted
            .authenticate_http(&inner.session, &parts.method, parts.headers)
            .await
        {
            Ok(authenticated) => authenticated,
            Err(failure) => {
                return failure
                    .into_response(inner.response.http)
                    .await
                    .map(Prepared::Response)
                    .map_err(Failure::Authorization);
            }
        };
        let prepared = match authenticated
            .prepare_http(&parts.method, &inner.accounts, inner.settings.limits)
            .await
        {
            Ok(prepared) => prepared,
            Err(failure) => {
                return failure
                    .into_response(http::account::Limits {
                        header_bytes: inner.response.account_headers,
                        body: inner.response.http,
                    })
                    .await
                    .map(Prepared::Response)
                    .map_err(Failure::Account);
            }
        };
        let selected = match prepared.select_json(&parts.method, rpc) {
            Ok(selected) => selected,
            Err(failure) => {
                return failure
                    .into_response(inner.response.http)
                    .await
                    .map(Prepared::Response)
                    .map_err(Failure::Selection);
            }
        };
        let operation = match selected.decode(body).await {
            Ok(operation) => operation,
            Err(failure) => {
                return failure
                    .into_response(inner.response.http)
                    .await
                    .map(Prepared::Response)
                    .map_err(Failure::Selection);
            }
        };
        Ok(Prepared::Ready(Box::new(Ready {
            inner: self.inner.clone(),
            operation,
        })))
    }
}

fn route(path: &str) -> Option<Rpc> {
    Some(match path {
        "/process.Process/Start" => Rpc::Start,
        "/process.Process/Connect" => Rpc::Connect,
        "/process.Process/List" => Rpc::List,
        "/process.Process/SendInput" => Rpc::SendInput,
        "/process.Process/CloseStdin" => Rpc::CloseStdin,
        "/process.Process/SendSignal" => Rpc::SendSignal,
        "/process.Process/Update" => Rpc::Update,
        _ => return None,
    })
}

impl<M: MetadataSource, S: AccountSource> Ready<M, S> {
    #[must_use]
    pub const fn request(&self) -> &protocol::Request {
        self.operation.request()
    }
    #[must_use]
    pub const fn deadline(&self) -> Instant {
        self.operation.deadline()
    }

    /// Dispatch the original request once under the same fixed resources. Start
    /// consumes a fresh internal command ID even if its preparation later fails.
    /// # Errors
    /// Original execution/response failure or exhausted ID sequence. Unknown
    /// faults remain owned typed failures; this component invents no public text.
    pub async fn execute(self) -> Result<Response, Failure> {
        let Self { inner, operation } = self;
        let reply = if matches!(operation.request(), protocol::Request::Start(_)) {
            let command = next_command(&inner.sequence).ok_or_else(|| {
                Failure::SequenceExhausted(Retained {
                    deadline: operation.deadline(),
                    _owner: operation.owner(),
                })
            })?;
            let p = &inner.processes;
            match operation
                .start_pid(
                    &inner.accounts,
                    &inner.settings,
                    StartPreparation {
                        owners: &p.owners,
                        registry: &p.registry,
                        registrar: &p.registrar,
                        binding: p.binding,
                        session: p.session,
                        command,
                        worker: &p.worker,
                        private_directory: &p.private_directory,
                        process_limits: p.process_limits,
                        admission_limits: p.admission_limits,
                    },
                    inner.response.message,
                )
                .await
            {
                Ok(reply) => reply,
                Err(failure) => {
                    return failure
                        .into_start_response(inner.response.http)
                        .await
                        .map_err(Failure::Start);
                }
            }
        } else {
            match operation
                .execute_pid(&inner.processes.registry, inner.response.message)
                .await
            {
                Ok(reply) => reply,
                Err(failure) => {
                    return failure
                        .into_operation_response(inner.response.http)
                        .await
                        .map_err(Failure::Operation);
                }
            }
        };
        reply
            .into_http_response(inner.response.http)
            .await
            .map_err(Failure::Response)
    }
}

pub struct Retained {
    deadline: Instant,
    _owner: Owner,
}
impl fmt::Debug for Retained {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RetainedCommandRequest")
            .finish_non_exhaustive()
    }
}
#[derive(Debug)]
pub enum Failure {
    Admission(command_requests::AdmissionError),
    Route(Retained),
    SequenceExhausted(Retained),
    Authorization(http::authorization::ReplyFailure),
    Account(http::account::ReplyFailure),
    Selection(http::reply::Failure),
    Start(http::start::Failure),
    Operation(http::operation::Failure),
    Response(http::response::Failure),
}
impl Failure {
    #[must_use]
    pub fn deadline(&self) -> Option<Instant> {
        Some(match self {
            Self::Admission(_) => return None,
            Self::Route(f) | Self::SequenceExhausted(f) => f.deadline,
            Self::Authorization(f) => f.deadline(),
            Self::Account(f) => f.deadline(),
            Self::Selection(f) => f.deadline(),
            Self::Start(f) => f.deadline(),
            Self::Operation(f) => f.deadline(),
            Self::Response(f) => f.deadline(),
        })
    }
}
impl fmt::Display for Failure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("command HTTP dispatch failed")
    }
}
impl std::error::Error for Failure {}

fn next_command(sequence: &AtomicU64) -> Option<u64> {
    sequence
        .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |n| n.checked_add(1))
        .ok()
        .map(|n| n + 1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shared_sequence_never_wraps_or_reissues_ids_at_exhaustion() {
        let sequence = AtomicU64::new(u64::MAX - 3);
        let mut ids = std::thread::scope(|scope| {
            let jobs: Vec<_> = (0..8)
                .map(|_| scope.spawn(|| next_command(&sequence)))
                .collect();
            jobs.into_iter()
                .filter_map(|job| job.join().unwrap())
                .collect::<Vec<_>>()
        });
        ids.sort_unstable();
        assert_eq!(ids, [u64::MAX - 2, u64::MAX - 1, u64::MAX]);
        assert_eq!(next_command(&sequence), None);
        assert_eq!(sequence.load(Ordering::Relaxed), u64::MAX);
    }
}
