//! Preparation owns the record reservation before spawn. The returned ticket
//! grants no public PID policy: only its caller can confirm the required
//! registration, and only then may the original worker be dispatched.
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use sandproof_guest_core::CommandIdentity;
use sandproof_guest_runtime::{
    CommandPreparation, CommandProcess, LaunchSpec, PreparationRejection, ProcessLimits,
};
use tokio::sync::{oneshot, watch};
use tokio::time::Instant;

use super::{
    AuthPermit, CommandError, CommandOwners, CommandReservation, CommandSubscription,
    ExecutionDeadline, ManagedCommand,
};

mod owner;

/// Trusted installation/context input, not an additional SDK request schema.
pub struct PreparationSpec {
    pub worker: PathBuf,
    pub private_directory: PathBuf,
    pub session: u64,
    pub command: u64,
    pub launch: LaunchSpec,
    pub process_limits: ProcessLimits,
}

/// Original request capacity and absolute deadline from the trusted admission
/// boundary. This does not authenticate or reset command execution time.
pub struct PreparationRequest {
    pub deadline: Instant,
    pub owner: Arc<dyn Send + Sync>,
}

struct Context {
    authorization: AuthPermit,
    execution: ExecutionDeadline,
    limits: AdmissionLimits,
    until: Instant,
    request: Arc<dyn Send + Sync>,
}

#[derive(Clone, Copy, Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AdmissionLimits {
    /// One absolute deadline covering preparation AND registration decision.
    pub ready_timeout: Duration,
    /// Bounds each cleanup observation; timeout retains the original worker.
    pub cleanup_timeout: Duration,
    /// Between unknown cleanup attempts; never forks a replacement worker.
    pub retry_delay: Duration,
}

impl AdmissionLimits {
    pub(crate) fn validate(self, start: Instant) -> Result<Self, CommandError> {
        if [self.ready_timeout, self.cleanup_timeout, self.retry_delay]
            .into_iter()
            .any(|duration| duration.is_zero() || start.checked_add(duration).is_none())
        {
            return Err(CommandError::InvalidArgument);
        }
        Ok(self)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AdmissionStatus {
    Preparing,
    Ready(CommandIdentity),
    /// Dispatch was accepted; start acknowledgement can still fail/be unknown.
    Dispatched(CommandIdentity),
    Stopping(CommandError),
    /// Wait failed/timed out. Original ownership/capacity is still retained.
    CleanupUnknown,
    /// No child was spawned or actual wait settled the original child.
    Stopped(CommandError),
    /// Owner disappeared. Any possibly spawned child keeps its slot quarantined.
    OwnerLost,
}

#[derive(Clone)]
pub struct AdmissionObserver(
    watch::Receiver<AdmissionStatus>,
    Arc<Mutex<Option<PreparationRejection>>>,
);

impl AdmissionObserver {
    // The immutable fact is published before Stopping. Unknown cleanup or
    // owner loss cannot be relabelled as this original preparation failure.
    pub(crate) fn rejection(&self) -> Option<PreparationRejection> {
        if !matches!(
            self.status(),
            AdmissionStatus::Stopping(CommandError::InvalidArgument)
                | AdmissionStatus::Stopped(CommandError::InvalidArgument)
        ) {
            return None;
        }
        self.1.lock().ok().and_then(|fact| *fact)
    }

    #[must_use]
    pub fn status(&self) -> AdmissionStatus {
        *self.0.borrow()
    }

    /// # Errors
    /// A closed owner channel never becomes a cleanup receipt.
    pub async fn changed(&mut self) -> Result<AdmissionStatus, CommandError> {
        self.0
            .changed()
            .await
            .map_err(|_| CommandError::Unavailable)?;
        Ok(*self.0.borrow_and_update())
    }
}

enum Decision {
    Commit(CommandIdentity),
    Abort,
}
type Launch = (ManagedCommand, CommandSubscription);
enum Reply {
    Dispatched(Launch),
    Stopped(CommandError),
    Unknown,
}

/// Single-use private registration ticket. Drop cancels PRE-dispatch work;
/// after dispatch acceptance the independent command owner remains responsible.
pub struct PendingCommand {
    observer: AdmissionObserver,
    decision: Option<oneshot::Sender<Decision>>,
    reply: oneshot::Receiver<Reply>,
    // The storage task owns this ticket. Even if preparation has already
    // stopped, queued/running durable PID work retains original request credit.
    _request: Arc<dyn Send + Sync>,
}

impl PendingCommand {
    /// An observer grants no signal or release capability and holds no slot.
    #[must_use]
    pub fn observer(&self) -> AdmissionObserver {
        self.observer.clone()
    }

    /// Return the exact still-gated child identity for registration. Cancellation
    /// of this borrow does not release it; dropping the whole ticket cancels.
    /// # Errors
    /// Invalid preparation, deadlines, stopping and owner loss are explicit.
    pub async fn prepared(&mut self) -> Result<CommandIdentity, CommandError> {
        loop {
            match self.observer.status() {
                AdmissionStatus::Ready(identity) => return Ok(identity),
                AdmissionStatus::Preparing => {
                    self.observer.changed().await?;
                }
                AdmissionStatus::Stopped(error) | AdmissionStatus::Stopping(error) => {
                    return Err(error);
                }
                _ => return Err(CommandError::Unavailable),
            }
        }
    }

    /// Confirm that the caller registered this exact child under the reviewed
    /// selector policy. No registration is inferred from this library call.
    /// Before acceptance, cancellation aborts preparation; after acceptance,
    /// loss of this response is unknown and never cancels/replays execution.
    /// # Errors
    /// Premature/stale confirmation, expired admission and owner/cleanup loss.
    pub async fn commit(mut self, identity: CommandIdentity) -> Result<Launch, CommandError> {
        match self.observer.status() {
            AdmissionStatus::Ready(actual) if actual == identity => {}
            AdmissionStatus::Ready(_) => return Err(CommandError::StaleIdentity),
            AdmissionStatus::Stopping(error) | AdmissionStatus::Stopped(error) => {
                return Err(error);
            }
            AdmissionStatus::Preparing => return Err(CommandError::NotStarted),
            _ => return Err(CommandError::Unavailable),
        }
        self.decision
            .take()
            .ok_or(CommandError::Unavailable)?
            .send(Decision::Commit(identity))
            .map_err(|_| CommandError::Unavailable)?;
        match self.reply.await.map_err(|_| CommandError::Unavailable)? {
            Reply::Dispatched(launch) => Ok(launch),
            Reply::Stopped(error) => Err(error),
            Reply::Unknown => Err(CommandError::Unavailable),
        }
    }

    /// Abort uncommitted admission and wait for actual cleanup. A timeout/error
    /// does not return capacity; the bounded owner continues the same cleanup.
    /// # Errors
    /// Returns Unavailable if cleanup cannot be confirmed or the owner is lost.
    pub async fn abort(mut self) -> Result<(), CommandError> {
        if let Some(decision) = self.decision.take() {
            let _ = decision.send(Decision::Abort);
        }
        match self.reply.await.map_err(|_| CommandError::Unavailable)? {
            Reply::Stopped(_) => Ok(()),
            Reply::Dispatched(_) | Reply::Unknown => Err(CommandError::Unavailable),
        }
    }
}

impl CommandOwners {
    /// Reserve before spawn and establish an independent preparation owner.
    /// Its ticket keeps the worker behind the private gate until registration
    /// is confirmed. Both deadlines retain their original absolute origins.
    /// # Errors
    /// Invalid timeout configuration or full active/retained record capacity.
    /// # Panics
    /// Requires a running Tokio runtime, as does the underlying process port.
    pub fn prepare_command(
        &self,
        spec: PreparationSpec,
        authorization: AuthPermit,
        execution: ExecutionDeadline,
        limits: AdmissionLimits,
    ) -> Result<PendingCommand, CommandError> {
        let deadline = Instant::now()
            .checked_add(limits.ready_timeout)
            .ok_or(CommandError::InvalidArgument)?;
        self.prepare_command_admitted(
            spec,
            authorization,
            execution,
            limits,
            PreparationRequest {
                deadline,
                owner: Arc::new(()),
            },
        )
    }

    /// Same original preparation/registration owner, retaining caller admission
    /// through actual preparation cleanup, persistent PID work and gate release.
    /// The request deadline can only shorten the preparation/decision deadline;
    /// after dispatch acceptance it does not replace the execution deadline.
    /// The caller separately retains a clone through response transport.
    /// # Errors
    /// Invalid limits, expired admission or full process record capacity.
    /// # Panics
    /// Requires a running Tokio runtime.
    pub fn prepare_command_admitted(
        &self,
        spec: PreparationSpec,
        authorization: AuthPermit,
        execution: ExecutionDeadline,
        limits: AdmissionLimits,
        request: PreparationRequest,
    ) -> Result<PendingCommand, CommandError> {
        let start = Instant::now();
        let limits = limits.validate(start)?;
        if start >= request.deadline {
            return Err(CommandError::DeadlineExceeded);
        }
        let until = request.deadline.min(start + limits.ready_timeout);
        let until = execution.0.map_or(until, |at| at.min(until));
        let reservation = self.reserve()?;
        let (status, observer) = watch::channel(AdmissionStatus::Preparing);
        let rejection = Arc::new(Mutex::new(None));
        let (decision, receive_decision) = oneshot::channel();
        let (reply, receive_reply) = oneshot::channel();
        owner::spawn(
            spec,
            Context {
                authorization,
                execution,
                limits,
                until,
                request: request.owner.clone(),
            },
            receive_decision,
            owner::AdmissionOwner::new(reservation, status, reply, rejection.clone()),
        );
        Ok(PendingCommand {
            observer: AdmissionObserver(observer, rejection),
            decision: Some(decision),
            reply: receive_reply,
            _request: request.owner,
        })
    }
}

enum Worker {
    Preparing(Box<CommandPreparation>),
    Ready(Box<CommandProcess>),
}

impl Worker {
    async fn abort(&mut self) -> std::io::Result<()> {
        match self {
            Self::Preparing(worker) => {
                worker.abort().await?;
            }
            Self::Ready(process) => {
                process.abort_prepared().await?;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejection_fact_does_not_relabel_expiry_owner_loss_or_unknown_cleanup() {
        let (status, receiver) = watch::channel(AdmissionStatus::Preparing);
        let fact = Arc::new(Mutex::new(None));
        let observer = AdmissionObserver(receiver, fact.clone());
        status.send_replace(AdmissionStatus::Stopped(CommandError::InvalidArgument));
        assert_eq!(observer.rejection(), None);
        for rejection in [
            PreparationRejection::MissingCwd,
            PreparationRejection::EnvironmentNul,
            PreparationRejection::ArgumentNul,
        ] {
            *fact.lock().unwrap() = Some(rejection);
            for state in [
                AdmissionStatus::Preparing,
                AdmissionStatus::Ready(CommandIdentity {
                    session: 1,
                    command: 1,
                    pid: 1,
                    start_ticks: 1,
                }),
                AdmissionStatus::Stopping(CommandError::DeadlineExceeded),
                AdmissionStatus::Stopped(CommandError::Cancelled),
                AdmissionStatus::Stopped(CommandError::Unavailable),
                AdmissionStatus::CleanupUnknown,
                AdmissionStatus::OwnerLost,
            ] {
                status.send_replace(state);
                assert_eq!(observer.rejection(), None, "{state:?}");
            }
            for state in [
                AdmissionStatus::Stopping(CommandError::InvalidArgument),
                AdmissionStatus::Stopped(CommandError::InvalidArgument),
            ] {
                status.send_replace(state);
                assert_eq!(observer.rejection(), Some(rejection));
            }
        }
    }
}
