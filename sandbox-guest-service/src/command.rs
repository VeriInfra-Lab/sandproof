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

mod driver;
mod preparation;
mod registration;
mod registry;
mod streams;
pub use preparation::{
    AdmissionLimits, AdmissionObserver, AdmissionStatus, PendingCommand, PreparationSpec,
};
pub use registration::{PidRegisteredCommand, PidRegistrar, RegistrationError};
pub use registry::{
    PidRegistry, RegisteredStart, RegistryError, RegistryLimits, RegistryReservation,
};
#[cfg(test)]
mod tests;
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CommandExit {
    pub code: i32,
    pub signal: Option<i32>,
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

    /// Parse the fixed Connect wire range, not a new SDK field. HTTP admission
    /// must call this once; parsing again on Connect must only bound that stream.
    /// # Errors
    /// Duplicate, empty, signed, nondecimal and >10-digit values are rejected.
    pub fn from_headers(headers: &HeaderMap, start: Instant) -> Result<Self, CommandError> {
        let mut values = headers.get_all("connect-timeout-ms").iter();
        let first = values.next();
        if values.next().is_some() {
            return Err(CommandError::InvalidArgument);
        }
        let Some(value) = first else {
            return Ok(Self(None));
        };
        let value = value.to_str().map_err(|_| CommandError::InvalidArgument)?;
        if value.is_empty() || value.len() > 10 || !value.bytes().all(|b| b.is_ascii_digit()) {
            return Err(CommandError::InvalidArgument);
        }
        let ms = value.parse().map_err(|_| CommandError::InvalidArgument)?;
        Self::from_duration(start, Duration::from_millis(ms))
    }
}

/// Explicit component limits, not production defaults or total RAM bounds.
/// A record slot covers its owner and every retained handle/subscription.
#[derive(Clone, Copy, Debug)]
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
        let identity = process.identity();
        let control = process.control();
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
            status,
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
            Arc::clone(&command.shared),
        );
        (command, output)
    }
}

struct Shared {
    identity: CommandIdentity,
    control: CommandControl,
    status: watch::Sender<CommandStatus>,
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
        self.check(identity)?;
        self.shared
            .control
            .signal(identity, signal)
            .map_err(|_| CommandError::Unavailable)
    }

    /// # Errors
    /// Invalid/stale, oversized or overlapping writes fail without queuing.
    /// Failure/cancellation can follow a delivered prefix; never replay it.
    pub async fn write_input(
        &self,
        identity: CommandIdentity,
        bytes: &[u8],
    ) -> Result<(), CommandError> {
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
        self.write_channel(identity, bytes, true).await
    }

    /// Resize the original terminal independently of a blocked input writer.
    /// # Errors
    /// Stale/not-started/completed command, wrong channel or actual ioctl error.
    pub fn resize_pty(&self, identity: CommandIdentity, size: PtySize) -> Result<(), CommandError> {
        self.check(identity)?;
        self.require_started()?;
        self.shared
            .control
            .resize_pty(identity, size)
            .map_err(|error| input_error(&error))
    }

    async fn write_channel(
        &self,
        identity: CommandIdentity,
        bytes: &[u8],
        pty: bool,
    ) -> Result<(), CommandError> {
        self.check(identity)?;
        if bytes.len() > self.shared.limits.stdin_bytes {
            return Err(CommandError::ResourceExhausted);
        }
        let mut input = self
            .shared
            .input
            .try_lock()
            .map_err(|_| CommandError::ResourceExhausted)?;
        self.require_started()?;
        let writer = input.as_mut().ok_or(CommandError::ClosedInput)?;
        tokio::select! {
            biased;
            () = input_finished(&self.shared) => {
                input.take();
                Err(CommandError::ClosedInput)
            }
            result = async {
                if pty { writer.write_pty(identity, bytes).await }
                else { writer.write_input(identity, bytes).await }
            } => result.map_err(|error| input_error(&error)),
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

fn input_error(error: &std::io::Error) -> CommandError {
    match error.kind() {
        std::io::ErrorKind::InvalidInput => CommandError::InvalidArgument,
        _ => CommandError::Unavailable,
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
