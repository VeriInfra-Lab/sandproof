//! Owned command execution, independent of an HTTP request. Durable PID claims
//! connect gated preparation to a bounded active/retained PID registry. Tag
//! arbitration, public HTTP routes and later-request authorization remain
//! separate; a raw numeric PID is never adopted as a new control capability.
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::http::HeaderMap;
use sandproof_guest_core::{Channel, CommandIdentity};
use sandproof_guest_runtime::{
    CommandControl, CommandInput, CommandProcess, ProcessSignal, PtySize,
};
use tokio::sync::{OwnedSemaphorePermit, Semaphore, watch};
use tokio::time::Instant;

use crate::AuthPermit;

mod body;
mod driver;
mod end;
mod preparation;
mod registration;
mod registry;
mod streams;
pub use preparation::{
    AdmissionLimits, AdmissionObserver, AdmissionStatus, PendingCommand, PreparationRequest,
    PreparationSpec,
};
pub use registration::{PidRegisteredCommand, PidRegistrar, RegistrationError};
pub use registry::{
    PidRegistry, RegisteredStart, RegistryError, RegistryLimits, RegistryReservation,
};
#[cfg(test)]
mod tests;
pub use body::{CommandBodyError, CommandBodyLimits};
pub use streams::{CommandOutput, CommandSubscription, OutputEvent};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CommandError {
    Cancelled,
    InvalidArgument,
    StaleIdentity,
    ResourceExhausted,
    NotStarted,
    ClosedInput,
    DeadlineExceeded,
    LaunchUnknown,
    ExecFailed(i32),
    Lagged,
    Unavailable,
}

/// A precise observation from the original input attempt. Existing component
/// callers retain their original coarse error; HTTP may consume the fact.
pub(crate) enum InputFailure {
    Command(CommandError),
    Native { error: CommandError, errno: i32 },
    StdinClosed,
    StdinDisabled,
}
impl InputFailure {
    pub(crate) const fn error(self) -> CommandError {
        match self {
            Self::Command(error) | Self::Native { error, .. } => error,
            Self::StdinClosed => CommandError::Unavailable,
            Self::StdinDisabled => CommandError::ClosedInput,
        }
    }
}
impl From<CommandError> for InputFailure {
    fn from(error: CommandError) -> Self {
        Self::Command(error)
    }
}

pub(crate) enum SignalFailure {
    Command(CommandError),
    Native(i32),
}
impl SignalFailure {
    pub(crate) const fn error(self) -> CommandError {
        match self {
            Self::Command(error) => error,
            Self::Native(_) => CommandError::Unavailable,
        }
    }
    fn observed(error: &std::io::Error) -> Self {
        error
            .raw_os_error()
            .filter(|errno| *errno > 0)
            .map_or(Self::Command(CommandError::Unavailable), Self::Native)
    }
}
impl From<CommandError> for SignalFailure {
    fn from(error: CommandError) -> Self {
        Self::Command(error)
    }
}

pub(crate) enum ResizeFailure {
    Command(CommandError),
    Native { error: CommandError, errno: i32 },
}
impl ResizeFailure {
    pub(crate) const fn error(self) -> CommandError {
        match self {
            Self::Command(error) | Self::Native { error, .. } => error,
        }
    }
    fn observed(error: &std::io::Error) -> Self {
        let coarse = input_error(error);
        error
            .raw_os_error()
            .filter(|errno| *errno > 0)
            .map_or(Self::Command(coarse), |errno| Self::Native {
                error: coarse,
                errno,
            })
    }
}
impl From<CommandError> for ResizeFailure {
    fn from(error: CommandError) -> Self {
        Self::Command(error)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CommandExit {
    pub code: i32,
    pub signal: Option<i32>,
    /// The kernel wait-status flag; not evidence that a core file was saved.
    pub core_dumped: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LaunchStatus {
    Pending,
    Started,
    Failed(CommandError),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DeadlineAction {
    KillSent,
    KillFailed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CommandStatus {
    pub launch: LaunchStatus,
    /// Complete only after actual wait and output settlement; an error never
    /// represents a successful complete stream or absence of descendants.
    pub completion: Option<Result<CommandExit, CommandError>>,
    /// Captured by the execution owner when wait and output settlement finish.
    /// Retention consumers must use this origin, not their later wake-up time.
    pub completed_at: Option<Instant>,
    pub deadline_action: Option<DeadlineAction>,
    /// The owner task disappeared before observing settlement. Its capacity is
    /// quarantined for this pool's lifetime; this is not a completion receipt.
    pub owner_lost: bool,
}

/// An absolute execution deadline captured once, before process preparation.
/// Reconnect/stream deadlines cannot replace it. None/zero means no deadline.
#[derive(Clone, Copy, Debug)]
pub struct ExecutionDeadline(Option<Instant>);

impl ExecutionDeadline {
    /// # Errors
    /// Rejects durations whose absolute deadline cannot be represented.
    pub fn from_duration(start: Instant, duration: Duration) -> Result<Self, CommandError> {
        if duration.is_zero() {
            return Ok(Self(None));
        }
        start
            .checked_add(duration)
            .map(|end| Self(Some(end)))
            .ok_or(CommandError::InvalidArgument)
    }

    /// Parse the fixed Start execution timeout within the Connect wire range.
    /// Selects the first field; empty/zero/negative means no execution deadline.
    /// HTTP admission must separately reject an expired request before effects:
    /// wire zero/negative already expires that context. Capture this once at
    /// Start's original execution origin, never again on a later Connect.
    /// # Errors
    /// Malformed signed decimal, >10-byte value or unrepresentable deadline.
    pub fn from_headers(headers: &HeaderMap, start: Instant) -> Result<Self, CommandError> {
        let duration = crate::connect_timeout::execution_duration(headers)
            .map_err(|_| CommandError::InvalidArgument)?;
        duration.map_or(Ok(Self(None)), |duration| {
            Self::from_duration(start, duration)
        })
    }
}

/// Explicit component limits, not production defaults or total RAM bounds.
/// A record slot covers its owner and every retained handle/subscription.
#[derive(Clone, Copy, Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OwnerLimits {
    pub records: usize,
    pub subscribers: usize,
    pub queued_chunks: usize,
    /// Per subscriber, includes a chunk already returned to its caller until
    /// that `CommandOutput` is dropped. Configure at least the process chunk size.
    pub output_bytes: usize,
    pub stdin_bytes: usize,
}

impl OwnerLimits {
    fn validate(self) -> Result<Self, CommandError> {
        if [
            self.records,
            self.subscribers,
            self.queued_chunks,
            self.output_bytes,
            self.stdin_bytes,
        ]
        .into_iter()
        .any(|n| n == 0 || n > Semaphore::MAX_PERMITS)
            || u32::try_from(self.output_bytes).is_err()
            || self
                .records
                .checked_mul(self.subscribers)
                .and_then(|n| n.checked_mul(self.output_bytes))
                .is_none_or(|n| n > isize::MAX as usize)
        {
            return Err(CommandError::InvalidArgument);
        }
        Ok(self)
    }
}

#[derive(Clone)]
pub struct CommandOwners {
    slots: Arc<Semaphore>,
    limits: OwnerLimits,
}

impl CommandOwners {
    /// # Errors
    /// Invalid budgets are refused before channels/tasks/processes exist.
    pub fn new(limits: OwnerLimits) -> Result<Self, CommandError> {
        let limits = limits.validate()?;
        Ok(Self {
            slots: Arc::new(Semaphore::new(limits.records)),
            limits,
        })
    }

    /// Reserve before preparing a worker. Before handoff, the caller must keep
    /// this reservation until preparation/abort has actually settled; dropping
    /// it alone does not prove low-level worker cleanup. No waiting queue.
    /// # Errors
    /// Full active/retained record capacity rejects new admission.
    pub fn reserve(&self) -> Result<CommandReservation, CommandError> {
        let permit = Arc::clone(&self.slots)
            .try_acquire_owned()
            .map_err(|_| CommandError::ResourceExhausted)?;
        Ok(CommandReservation {
            permit,
            limits: self.limits,
        })
    }
}

pub struct CommandReservation {
    permit: OwnedSemaphorePermit,
    limits: OwnerLimits,
}

impl CommandReservation {
    /// Take ownership of an already prepared AND privately registered worker.
    /// The caller must establish public-selector ownership before this call.
    /// One task owns release, wait, pipes and expiry; dropping its handles does
    /// not cancel it. Authorization is retained through release, not execution.
    /// The initial output subscription is installed BEFORE spawning the task,
    /// including on multithreaded runtimes; a fast command cannot outrun Start.
    /// # Panics
    /// Requires a running Tokio runtime. Fresh validated budgets guarantee that
    /// the initial subscription can be reserved before any concurrent access.
    #[must_use]
    pub fn start_registered(
        self,
        process: CommandProcess,
        authorization: AuthPermit,
        deadline: ExecutionDeadline,
    ) -> (ManagedCommand, CommandSubscription) {
        self.start_registered_admitted(process, authorization, deadline, Arc::new(()))
    }

    fn start_registered_admitted(
        self,
        process: CommandProcess,
        authorization: AuthPermit,
        deadline: ExecutionDeadline,
        request: Arc<dyn Send + Sync>,
    ) -> (ManagedCommand, CommandSubscription) {
        let identity = process.identity();
        let control = process.control();
        let stdin_disabled = !control.has_pty() && process.has_closed_input();
        let (status, _) = watch::channel(CommandStatus {
            launch: LaunchStatus::Pending,
            completion: None,
            completed_at: None,
            deadline_action: None,
            owner_lost: false,
        });
        let shared = Arc::new(Shared {
            identity,
            control,
            stdin_disabled,
            status,
            output_wait_cancelled: watch::channel(false).0,
            streams: Mutex::new(streams::Subscribers::default()),
            subscriber_slots: Arc::new(Semaphore::new(self.limits.subscribers)),
            input: tokio::sync::Mutex::new(None),
            limits: self.limits,
            record: Mutex::new(Some(self.permit)),
        });
        let command = ManagedCommand { shared };
        let output = command
            .subscribe(identity)
            .expect("fresh validated subscriber budget");
        driver::spawn(
            process,
            authorization,
            deadline,
            request,
            Arc::clone(&command.shared),
        );
        (command, output)
    }
}

struct Shared {
    identity: CommandIdentity,
    control: CommandControl,
    stdin_disabled: bool,
    status: watch::Sender<CommandStatus>,
    output_wait_cancelled: watch::Sender<bool>,
    streams: Mutex<streams::Subscribers>,
    subscriber_slots: Arc<Semaphore>,
    input: tokio::sync::Mutex<Option<CommandInput>>,
    limits: OwnerLimits,
    record: Mutex<Option<OwnedSemaphorePermit>>,
}

/// Internal capability; public requests must first reauthenticate and resolve
/// a reviewed registry entry. Clones do not create tasks or change deadlines.
#[derive(Clone)]
pub struct ManagedCommand {
    shared: Arc<Shared>,
}

impl ManagedCommand {
    pub(crate) fn has_pty(&self) -> bool {
        self.shared.control.has_pty()
    }

    #[must_use]
    pub fn identity(&self) -> CommandIdentity {
        self.shared.identity
    }

    #[must_use]
    pub fn status(&self) -> CommandStatus {
        *self.shared.status.borrow()
    }

    /// # Errors
    /// A failed/unknown launch stays failed/unknown; waiting never retries it.
    pub async fn started(&self) -> Result<(), CommandError> {
        let mut status = self.shared.status.subscribe();
        loop {
            if status.borrow().owner_lost {
                return Err(CommandError::Unavailable);
            }
            match status.borrow_and_update().launch {
                LaunchStatus::Started => return Ok(()),
                LaunchStatus::Failed(error) => return Err(error),
                LaunchStatus::Pending => {}
            }
            status
                .changed()
                .await
                .map_err(|_| CommandError::Unavailable)?;
        }
    }

    /// # Errors
    /// Stream/launch failures never turn into a successful exit. Cancelling a
    /// wait affects only that waiter; it neither kills nor resets the command.
    pub async fn wait(&self) -> Result<CommandExit, CommandError> {
        let mut status = self.shared.status.subscribe();
        loop {
            if status.borrow().owner_lost {
                return Err(CommandError::Unavailable);
            }
            if let Some(result) = status.borrow_and_update().completion {
                return result;
            }
            status
                .changed()
                .await
                .map_err(|_| CommandError::Unavailable)?;
        }
    }

    /// # Errors
    /// Rejects stale identities and actual pidfd errors, independently of
    /// output/input pressure. Success reports delivery, never confirmed exit.
    pub fn signal(
        &self,
        identity: CommandIdentity,
        signal: ProcessSignal,
    ) -> Result<(), CommandError> {
        self.signal_observed(identity, signal)
            .map_err(SignalFailure::error)
    }

    pub(crate) fn signal_observed(
        &self,
        identity: CommandIdentity,
        signal: ProcessSignal,
    ) -> Result<(), SignalFailure> {
        self.check(identity)?;
        // Fixed Go cancels output waiting BEFORE attempting TERM/KILL, even
        // if the original process has already exited and signalling fails.
        // It keeps reading until actual main-child wait, not merely delivery.
        self.shared.output_wait_cancelled.send_replace(true);
        self.shared
            .control
            .signal(identity, signal)
            .map_err(|error| SignalFailure::observed(&error))
    }

    /// # Errors
    /// Invalid/stale, oversized or overlapping writes fail without queuing.
    /// Failure/cancellation can follow a delivered prefix; never replay it.
    pub async fn write_input(
        &self,
        identity: CommandIdentity,
        bytes: &[u8],
    ) -> Result<(), CommandError> {
        self.write_input_observed(identity, bytes)
            .await
            .map_err(InputFailure::error)
    }

    pub(crate) async fn write_input_observed(
        &self,
        identity: CommandIdentity,
        bytes: &[u8],
    ) -> Result<(), InputFailure> {
        self.write_channel(identity, bytes, false).await
    }

    /// Write terminal-channel bytes with the same independent input budget.
    /// # Errors
    /// Stale/wrong channel, closed input, oversized/overlapping write or I/O.
    pub async fn write_pty(
        &self,
        identity: CommandIdentity,
        bytes: &[u8],
    ) -> Result<(), CommandError> {
        self.write_pty_observed(identity, bytes)
            .await
            .map_err(InputFailure::error)
    }

    pub(crate) async fn write_pty_observed(
        &self,
        identity: CommandIdentity,
        bytes: &[u8],
    ) -> Result<(), InputFailure> {
        self.write_channel(identity, bytes, true).await
    }

    /// Resize the original terminal independently of a blocked input writer.
    /// # Errors
    /// Stale/not-started/completed command, wrong channel or actual ioctl error.
    pub fn resize_pty(&self, identity: CommandIdentity, size: PtySize) -> Result<(), CommandError> {
        self.resize_pty_observed(identity, size)
            .map_err(ResizeFailure::error)
    }

    pub(crate) fn resize_pty_observed(
        &self,
        identity: CommandIdentity,
        size: PtySize,
    ) -> Result<(), ResizeFailure> {
        self.check(identity)?;
        self.require_started()?;
        self.shared
            .control
            .resize_pty(identity, size)
            .map_err(|error| ResizeFailure::observed(&error))
    }

    async fn write_channel(
        &self,
        identity: CommandIdentity,
        bytes: &[u8],
        pty: bool,
    ) -> Result<(), InputFailure> {
        self.check(identity)?;
        if bytes.len() > self.shared.limits.stdin_bytes {
            return Err(CommandError::ResourceExhausted.into());
        }
        let mut input = self
            .shared
            .input
            .try_lock()
            .map_err(|_| CommandError::ResourceExhausted)?;
        self.require_started()?;
        let writer = input.as_mut().ok_or_else(|| {
            if !pty && self.shared.stdin_disabled {
                InputFailure::StdinDisabled
            } else {
                InputFailure::Command(CommandError::ClosedInput)
            }
        })?;
        tokio::select! {
            biased;
            () = input_finished(&self.shared) => {
                input.take();
                Err(CommandError::ClosedInput.into())
            }
            result = async {
                // Borrowed under the original exclusive writer lock; the
                // actual attempt still runs once. No later PID/state lookup
                // may infer this reason from a generic BrokenPipe/Unavailable.
                let closed_stdin = !pty && !self.has_pty() && writer.is_closed();
                let result = if pty { writer.write_pty(identity, bytes).await }
                    else { writer.write_input(identity, bytes).await };
                result.map_err(|error| input_failure(&error, closed_stdin))
            } => result,
        }
    }

    /// # Errors
    /// A concurrent writer is explicitly busy. Repeated close while the command
    /// is active is accepted; dropping a subscription never invokes this method.
    pub fn close_input(&self, identity: CommandIdentity) -> Result<(), CommandError> {
        self.check(identity)?;
        let mut input = self
            .shared
            .input
            .try_lock()
            .map_err(|_| CommandError::ResourceExhausted)?;
        self.require_started()?;
        if let Some(input) = input.as_mut() {
            input
                .close_input(identity)
                .map_err(|error| input_error(&error))?;
        }
        Ok(())
    }

    fn require_started(&self) -> Result<(), CommandError> {
        let status = self.status();
        if status.owner_lost {
            return Err(CommandError::Unavailable);
        }
        if status.completion.is_some() {
            return Err(CommandError::ClosedInput);
        }
        match status.launch {
            LaunchStatus::Started => Ok(()),
            LaunchStatus::Pending => Err(CommandError::NotStarted),
            LaunchStatus::Failed(error) => Err(error),
        }
    }

    fn check(&self, identity: CommandIdentity) -> Result<(), CommandError> {
        if identity == self.identity() {
            Ok(())
        } else {
            Err(CommandError::StaleIdentity)
        }
    }
}

pub(crate) fn input_error(error: &std::io::Error) -> CommandError {
    match error.kind() {
        std::io::ErrorKind::InvalidInput => CommandError::InvalidArgument,
        _ => CommandError::Unavailable,
    }
}

// Called only for the completed original write, while holding its writer lock.
// Synthetic mode/identity errors and cancellation never become native facts.
fn input_failure(error: &std::io::Error, closed_stdin: bool) -> InputFailure {
    if closed_stdin && error.kind() == std::io::ErrorKind::BrokenPipe {
        InputFailure::StdinClosed
    } else if let Some(errno) = error.raw_os_error().filter(|errno| *errno > 0) {
        InputFailure::Native {
            error: input_error(error),
            errno,
        }
    } else {
        InputFailure::Command(input_error(error))
    }
}

async fn input_finished(shared: &Shared) {
    let mut status = shared.status.subscribe();
    loop {
        let state = *status.borrow_and_update();
        if state.completion.is_some() || state.owner_lost {
            return;
        }
        if status.changed().await.is_err() {
            return;
        }
    }
}
