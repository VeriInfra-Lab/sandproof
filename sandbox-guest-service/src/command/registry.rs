//! Bounded active/retained PID records. This component consumes a durable claim
//! before the original preparation ticket can dispatch. It does not implement
//! tag arbitration, HTTP error mapping or per-request authorization.
use super::{
    AdmissionObserver, AdmissionStatus, CommandError, CommandSubscription, LaunchStatus,
    ManagedCommand, PidRegisteredCommand,
};
use sandproof_guest_core::CommandIdentity;
use sandproof_guest_runtime::{
    pid_claims::Binding,
    protocol::{ProcessConfig, ProcessInfo, Response, encode_response},
};
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex, PoisonError},
    time::Duration,
};
use tokio::{
    sync::{OwnedSemaphorePermit, Semaphore, oneshot},
    time::Instant,
};

#[derive(Clone, Copy, Debug)]
pub struct RegistryLimits {
    /// Includes reservations, live records, retained exits and unknown owners.
    pub records: usize,
    /// Maximum encoded single-record List reply, including config and tag.
    /// String/container allocator overhead is separate from this wire budget.
    pub metadata_bytes: usize,
    /// Explicit component retention. The fixed Go endpoint uses 30 seconds;
    /// the eventual public assembly must select its reviewed policy.
    pub retention: Duration,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RegistryError {
    InvalidArgument,
    ResourceExhausted,
    StaleIdentity,
    NotFound,
    NotReady,
    Unavailable,
}

#[derive(Clone)]
pub struct PidRegistry {
    inner: Arc<Inner>,
}
struct Inner {
    binding: Binding,
    session: u64,
    limits: RegistryLimits,
    slots: Arc<Semaphore>,
    records: Mutex<BTreeMap<u32, Arc<Record>>>,
}
struct Record {
    identity: CommandIdentity,
    metadata: Metadata,
    state: Mutex<State>,
    _slot: OwnedSemaphorePermit,
}
struct Metadata {
    config: ProcessConfig,
    tag: Option<String>,
}
#[derive(Default)]
struct State {
    command: Option<ManagedCommand>,
    unknown: bool,
}

impl PidRegistry {
    /// One trusted VM binding and service session; no production defaults.
    /// # Errors
    /// Invalid/overflowing budgets or zero session, before any task is started.
    pub fn new(
        binding: Binding,
        session: u64,
        limits: RegistryLimits,
    ) -> Result<Self, RegistryError> {
        if session == 0
            || limits.records == 0
            || limits.records > Semaphore::MAX_PERMITS
            || limits.metadata_bytes == 0
            || limits.retention.is_zero()
            || limits
                .records
                .checked_mul(limits.metadata_bytes)
                .is_none_or(|n| n > isize::MAX as usize)
            || Instant::now().checked_add(limits.retention).is_none()
        {
            return Err(RegistryError::InvalidArgument);
        }
        Ok(Self {
            inner: Arc::new(Inner {
                binding,
                session,
                limits,
                slots: Arc::new(Semaphore::new(limits.records)),
                records: Mutex::new(BTreeMap::new()),
            }),
        })
    }

    /// Reserve registry capacity before preparing the worker. Metadata must be
    /// the trusted caller's original public command configuration; no daemon
    /// environment or credential is inferred here. Tag is opaque List metadata,
    /// NOT a lookup capability or a duplicate-tag decision.
    /// # Errors
    /// Full capacity or metadata that cannot fit in its encoded record budget.
    pub fn reserve(
        &self,
        config: &ProcessConfig,
        tag: Option<&str>,
    ) -> Result<RegistryReservation, RegistryError> {
        let slot = self
            .inner
            .slots
            .clone()
            .try_acquire_owned()
            .map_err(|_| RegistryError::ResourceExhausted)?;
        // Include the widest possible public PID before accepting metadata, so
        // later registration cannot increase the promised record wire size.
        let entries = [ProcessInfo {
            config: Some(config),
            pid: u32::MAX,
            tag,
        }];
        encode_response(Response::List(&entries), self.inner.limits.metadata_bytes)
            .map_err(|_| RegistryError::ResourceExhausted)?;
        Ok(RegistryReservation {
            inner: self.inner.clone(),
            slot,
            metadata: Metadata {
                config: config.clone(),
                tag: tag.map(str::to_owned),
            },
        })
    }

    /// Snapshot only; normal reserve never queues or relies on this observation.
    #[must_use]
    pub fn available_slots(&self) -> usize {
        self.inner.slots.available_permits()
    }

    /// Resolve only an original started, nonterminal process. The dispatcher
    /// must independently reauthenticate every request before using the result.
    /// # Errors
    /// Unknown/missing/expired identity, pending launch or failed owner.
    pub fn live(&self, pid: u32) -> Result<ManagedCommand, RegistryError> {
        self.lookup(pid, false)
    }

    /// Resolve the original command, including its retained terminal event.
    /// Subscribing replays no prior output and never resets execution/retention.
    /// # Errors
    /// Same lookup refusals as live; expiry is checked even if its timer is late.
    pub fn connectable(&self, pid: u32) -> Result<ManagedCommand, RegistryError> {
        self.lookup(pid, true)
    }

    fn lookup(&self, pid: u32, retained: bool) -> Result<ManagedCommand, RegistryError> {
        let records = self
            .inner
            .records
            .lock()
            .map_err(|_| RegistryError::Unavailable)?;
        let record = records.get(&pid).ok_or(RegistryError::NotFound)?;
        let state = record
            .state
            .lock()
            .map_err(|_| RegistryError::Unavailable)?;
        if state.unknown {
            return Err(RegistryError::Unavailable);
        }
        let command = state.command.as_ref().ok_or(RegistryError::NotReady)?;
        let status = command.status();
        if status.owner_lost
            || matches!(status.launch, LaunchStatus::Failed(_))
            || status.completion.is_some_and(|result| result.is_err())
        {
            return Err(RegistryError::Unavailable);
        }
        if status.launch == LaunchStatus::Pending {
            return Err(RegistryError::NotReady);
        }
        if status.completion.is_some() {
            let end = status
                .completed_at
                .and_then(|at| at.checked_add(self.inner.limits.retention))
                .ok_or(RegistryError::Unavailable)?;
            if !retained || Instant::now() >= end {
                return Err(RegistryError::NotFound);
            }
        }
        Ok(command.clone())
    }

    /// Encode currently observed live entries with a separate complete-reply
    /// budget. Holds only the bounded table during synchronous serialization;
    /// no process, filesystem or network work runs under this lock.
    /// An empty list never proves that all guest processes or descendants stopped.
    /// # Errors
    /// Unavailable table/owner state or an insufficient full response budget.
    pub fn list_json(&self, bytes: usize) -> Result<Vec<u8>, RegistryError> {
        let records = self
            .inner
            .records
            .lock()
            .map_err(|_| RegistryError::Unavailable)?;
        let mut entries = Vec::new();
        entries
            .try_reserve_exact(records.len())
            .map_err(|_| RegistryError::ResourceExhausted)?;
        for record in records.values() {
            let state = record
                .state
                .lock()
                .map_err(|_| RegistryError::Unavailable)?;
            if state.unknown {
                continue;
            }
            if let Some(command) = &state.command {
                let status = command.status();
                if !status.owner_lost
                    && status.launch == LaunchStatus::Started
                    && status.completion.is_none()
                {
                    entries.push(ProcessInfo {
                        config: Some(&record.metadata.config),
                        pid: record.identity.pid,
                        tag: record.metadata.tag.as_deref(),
                    });
                }
            }
        }
        encode_response(Response::List(&entries), bytes)
            .map_err(|_| RegistryError::ResourceExhausted)
    }
}

pub struct RegistryReservation {
    inner: Arc<Inner>,
    metadata: Metadata,
    slot: OwnedSemaphorePermit,
}
pub struct RegisteredStart {
    identity: CommandIdentity,
    reply: oneshot::Receiver<Result<(ManagedCommand, CommandSubscription), CommandError>>,
}
impl RegisteredStart {
    #[must_use]
    pub const fn identity(&self) -> CommandIdentity {
        self.identity
    }

    /// Wait for the original launch observation and initial subscription. Drop
    /// before owner acceptance aborts preparation; afterwards it only loses the
    /// reply/subscription. The registry keeps its original control capability.
    /// # Errors
    /// Launch/cleanup uncertainty never triggers another execution.
    pub async fn started(self) -> Result<(ManagedCommand, CommandSubscription), CommandError> {
        self.reply.await.map_err(|_| CommandError::Unavailable)?
    }
}

impl RegistryReservation {
    /// Install the durable exact identity before dispatching the original child.
    /// No raw PID adoption and no caller-controlled replacement of the claim.
    /// # Errors
    /// Wrong VM/session or an already registered PID; the ticket is cancelled.
    /// # Panics
    /// Requires a running Tokio runtime.
    pub fn publish(self, ticket: PidRegisteredCommand) -> Result<RegisteredStart, RegistryError> {
        let identity = ticket.identity();
        if identity.session != self.inner.session || !ticket.belongs_to(self.inner.binding) {
            return Err(RegistryError::StaleIdentity);
        }
        let record = Arc::new(Record {
            identity,
            metadata: self.metadata,
            state: Mutex::new(State::default()),
            _slot: self.slot,
        });
        {
            let mut records = self
                .inner
                .records
                .lock()
                .map_err(|_| RegistryError::Unavailable)?;
            if records.contains_key(&identity.pid) {
                return Err(RegistryError::StaleIdentity);
            }
            records.insert(identity.pid, record.clone());
        }
        let (reply, receive) = oneshot::channel();
        // Created before spawning: even an unpolled/aborted task leaves an
        // unavailable record occupying its original slot, not false credit.
        let guard = Guard {
            inner: self.inner,
            record,
            removed: false,
        };
        tokio::spawn(run(ticket, reply, guard));
        Ok(RegisteredStart {
            identity,
            reply: receive,
        })
    }
}

struct Guard {
    inner: Arc<Inner>,
    record: Arc<Record>,
    removed: bool,
}
impl Guard {
    fn remove(&mut self) -> Result<(), RegistryError> {
        let mut records = self
            .inner
            .records
            .lock()
            .map_err(|_| RegistryError::Unavailable)?;
        if records
            .get(&self.record.identity.pid)
            .is_some_and(|actual| Arc::ptr_eq(actual, &self.record))
        {
            records.remove(&self.record.identity.pid);
        }
        self.removed = true;
        Ok(())
    }
}
impl Drop for Guard {
    fn drop(&mut self) {
        if !self.removed {
            self.record
                .state
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .unknown = true;
        }
    }
}

async fn stopped(observer: &mut AdmissionObserver) -> bool {
    loop {
        match observer.status() {
            AdmissionStatus::Stopped(_) => return true,
            AdmissionStatus::Dispatched(_) | AdmissionStatus::OwnerLost => return false,
            _ => {}
        }
        if observer.changed().await.is_err() {
            return false;
        }
    }
}

async fn run(
    ticket: PidRegisteredCommand,
    reply: oneshot::Sender<Result<(ManagedCommand, CommandSubscription), CommandError>>,
    mut guard: Guard,
) {
    let mut observer = ticket.observer();
    if reply.is_closed() {
        let _ = ticket.abort().await;
        if stopped(&mut observer).await {
            let _ = guard.remove();
        }
        return;
    }
    // Acceptance: the independent registry owner, not the HTTP waiter, holds
    // this commit future until its result is installed. Lost replies cannot
    // discard the only control handle or cause a new Start.
    let (command, output) = match ticket.commit().await {
        Ok(launch) => launch,
        Err(error) => {
            let _ = reply.send(Err(error));
            if stopped(&mut observer).await {
                let _ = guard.remove();
            }
            return;
        }
    };
    if let Ok(mut state) = guard.record.state.lock() {
        state.command = Some(command.clone());
    } else {
        let _ = reply.send(Err(CommandError::Unavailable));
        return;
    }
    match command.started().await {
        Ok(()) => {
            let _ = reply.send(Ok((command.clone(), output)));
        }
        Err(error) => {
            drop(output);
            let _ = reply.send(Err(error));
        }
    }
    let _ = command.wait().await;
    let status = command.status();
    if status.owner_lost || status.completion.is_none() {
        return;
    }
    // Failed launches/streams are never cached as successful terminal events.
    if status.completion.is_some_and(|result| result.is_ok()) {
        let Some(expiry) = status
            .completed_at
            .and_then(|at| at.checked_add(guard.inner.limits.retention))
        else {
            return;
        };
        tokio::time::sleep_until(expiry).await;
    }
    let _ = guard.remove();
}

#[cfg(test)]
mod tests;
