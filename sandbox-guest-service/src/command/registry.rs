//! Bounded active/retained PID records. This component consumes a durable claim
//! before the original preparation ticket can dispatch. Tags are reserved
//! before preparation and resolve deterministically within this service session.
//! HTTP error mapping and per-request authorization belong to the caller.
use super::{
    AdmissionObserver, AdmissionStatus, CommandError, CommandSubscription, LaunchStatus,
    ManagedCommand, PidRegisteredCommand,
};
use crate::task_accounting::{self, AsyncTask};
use sandproof_guest_core::CommandIdentity;
use sandproof_guest_runtime::{
    pid_claims::Binding,
    protocol::{ProcessConfig, ProcessInfo, Response, encode_response},
};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{Arc, Mutex, PoisonError},
    time::Duration,
};
use tokio::{
    sync::{Notify, OwnedSemaphorePermit, Semaphore, oneshot},
    time::Instant,
};

#[derive(Clone, Copy, Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
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
impl RegistryLimits {
    pub(crate) fn validate_public(self) -> Result<Self, RegistryError> {
        if self.retention != Duration::from_secs(30) {
            return Err(RegistryError::InvalidArgument);
        }
        self.validate()
    }

    pub(crate) fn validate(self) -> Result<Self, RegistryError> {
        if self.records == 0
            || self.records > Semaphore::MAX_PERMITS
            || self.metadata_bytes == 0
            || self.retention.is_zero()
            || self
                .records
                .checked_mul(self.metadata_bytes)
                .is_none_or(|n| n > isize::MAX as usize)
            || Instant::now().checked_add(self.retention).is_none()
        {
            return Err(RegistryError::InvalidArgument);
        }
        Ok(self)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RegistryError {
    InvalidArgument,
    ResourceExhausted,
    AlreadyExists,
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
    table: Mutex<Table>,
}
#[derive(Default)]
struct Table {
    records: BTreeMap<u32, Arc<Record>>,
    pending_tags: BTreeSet<Arc<str>>,
}
struct Record {
    identity: CommandIdentity,
    metadata: Metadata,
    state: Mutex<State>,
    retired: Notify,
    _slot: Arc<OwnedSemaphorePermit>,
}
struct Metadata {
    config: Option<ProcessConfig>,
    tag: Option<String>,
}
#[derive(Default)]
struct State {
    command: Option<ManagedCommand>,
    unknown: bool,
}

impl PidRegistry {
    pub(crate) fn has_public_retention(&self) -> bool {
        self.inner.limits.validate_public().is_ok()
    }

    pub(crate) fn matches_session(&self, binding: Binding, session: u64) -> bool {
        self.inner.binding == binding && self.inner.session == session
    }

    /// One trusted VM binding and service session; no production defaults.
    /// # Errors
    /// Invalid/overflowing budgets or zero session, before any task is started.
    pub fn new(
        binding: Binding,
        session: u64,
        limits: RegistryLimits,
    ) -> Result<Self, RegistryError> {
        if session == 0 {
            return Err(RegistryError::InvalidArgument);
        }
        let limits = limits.validate()?;
        Ok(Self {
            inner: Arc::new(Inner {
                binding,
                session,
                limits,
                slots: Arc::new(Semaphore::new(limits.records)),
                table: Mutex::new(Table::default()),
            }),
        })
    }

    /// Reserve registry capacity before preparing the worker. Metadata must be
    /// the trusted caller's original public command configuration; no daemon
    /// environment or credential is inferred here. A present tag (including the
    /// empty string) excludes another reservation or live command with that tag.
    /// Dropping a reservation releases only its tag claim and capacity; an older
    /// retained result is cleared only when the new durable ticket is published.
    /// # Errors
    /// Duplicate/unknown tag, full capacity or oversized encoded metadata.
    pub fn reserve(
        &self,
        config: &ProcessConfig,
        tag: Option<&str>,
    ) -> Result<RegistryReservation, RegistryError> {
        self.reserve_optional(Some(config), tag)
    }

    // The original Start process field may be absent. Preserve that presence
    // separately from the empty command used by Go getters for execution.
    pub(crate) fn reserve_optional(
        &self,
        config: Option<&ProcessConfig>,
        tag: Option<&str>,
    ) -> Result<RegistryReservation, RegistryError> {
        // Include the widest possible public PID before accepting metadata, so
        // later registration cannot increase the promised record wire size.
        let entries = [ProcessInfo {
            config,
            pid: u32::MAX,
            tag,
        }];
        encode_response(Response::List(&entries), self.inner.limits.metadata_bytes)
            .map_err(|_| RegistryError::ResourceExhausted)?;
        let metadata = Metadata {
            config: config.cloned(),
            tag: tag.map(str::to_owned),
        };
        let mut table = self
            .inner
            .table
            .lock()
            .map_err(|_| RegistryError::Unavailable)?;
        if let Some(tag) = tag {
            if table.pending_tags.contains(tag) {
                return Err(RegistryError::AlreadyExists);
            }
            if let Some(record) = tagged(&table, tag)?
                && !record.finished()?
            {
                return Err(RegistryError::AlreadyExists);
            }
        }
        let slot = Arc::new(
            self.inner
                .slots
                .clone()
                .try_acquire_owned()
                .map_err(|_| RegistryError::ResourceExhausted)?,
        );
        let tag_claim = tag.map(|tag| {
            let tag: Arc<str> = tag.into();
            table.pending_tags.insert(tag.clone());
            TagClaim {
                inner: self.inner.clone(),
                tag: Some(tag),
                _slot: slot.clone(),
            }
        });
        Ok(RegistryReservation {
            inner: self.inner.clone(),
            slot,
            tag_claim,
            metadata,
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

    /// Resolve the unique live command with this exact tag. Unknown owners and
    /// pending starts never fall back to a different record or numeric PID.
    /// # Errors
    /// The same identity/state refusals as `live`, or an absent tag.
    pub fn live_tag(&self, tag: &str) -> Result<ManagedCommand, RegistryError> {
        self.lookup_tag(tag, false)
    }

    /// Resolve the original live or retained command with this exact tag.
    /// Lookup does not refresh retention; a new published use clears the old
    /// record for both PID and tag lookup. Existing handles keep their ownership.
    /// # Errors
    /// The same identity/state refusals as `connectable`, or an absent tag.
    pub fn connectable_tag(&self, tag: &str) -> Result<ManagedCommand, RegistryError> {
        self.lookup_tag(tag, true)
    }

    fn lookup_tag(&self, tag: &str, retained: bool) -> Result<ManagedCommand, RegistryError> {
        let table = self
            .inner
            .table
            .lock()
            .map_err(|_| RegistryError::Unavailable)?;
        let record = tagged(&table, tag)?.ok_or_else(|| {
            if table.pending_tags.contains(tag) {
                RegistryError::NotReady
            } else {
                RegistryError::NotFound
            }
        })?;
        self.lookup_record(record, retained)
    }

    fn lookup(&self, pid: u32, retained: bool) -> Result<ManagedCommand, RegistryError> {
        let table = self
            .inner
            .table
            .lock()
            .map_err(|_| RegistryError::Unavailable)?;
        let record = table.records.get(&pid).ok_or(RegistryError::NotFound)?;
        self.lookup_record(record, retained)
    }

    fn lookup_record(
        &self,
        record: &Record,
        retained: bool,
    ) -> Result<ManagedCommand, RegistryError> {
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
        let table = self
            .inner
            .table
            .lock()
            .map_err(|_| RegistryError::Unavailable)?;
        let mut entries = Vec::new();
        entries
            .try_reserve_exact(table.records.len())
            .map_err(|_| RegistryError::ResourceExhausted)?;
        for record in table.records.values() {
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
                        config: record.metadata.config.as_ref(),
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
    slot: Arc<OwnedSemaphorePermit>,
    tag_claim: Option<TagClaim>,
}

struct TagClaim {
    inner: Arc<Inner>,
    tag: Option<Arc<str>>,
    // Keep capacity even if publication drops its new record before this
    // claim's destructor can acquire the table lock and remove the tag.
    _slot: Arc<OwnedSemaphorePermit>,
}
impl TagClaim {
    fn release(&mut self, table: &mut Table) {
        if let Some(tag) = self.tag.take() {
            table.pending_tags.remove(&tag);
        }
    }
}
impl Drop for TagClaim {
    fn drop(&mut self) {
        if self.tag.is_some() {
            let inner = self.inner.clone();
            // Poison remains observable to subsequent operations. Drop may only
            // release this unconsumed claim; it never repairs an active record.
            self.release(&mut inner.table.lock().unwrap_or_else(PoisonError::into_inner));
        }
    }
}

fn tagged<'a>(table: &'a Table, tag: &str) -> Result<Option<&'a Arc<Record>>, RegistryError> {
    let mut matching = table
        .records
        .values()
        .filter(|record| record.metadata.tag.as_deref() == Some(tag));
    let record = matching.next();
    if matching.next().is_some() {
        return Err(RegistryError::Unavailable);
    }
    Ok(record)
}
impl Record {
    fn finished(&self) -> Result<bool, RegistryError> {
        let state = self.state.lock().map_err(|_| RegistryError::Unavailable)?;
        if state.unknown {
            return Err(RegistryError::Unavailable);
        }
        let Some(command) = &state.command else {
            return Ok(false);
        };
        let status = command.status();
        if status.owner_lost
            || matches!(status.launch, LaunchStatus::Failed(_))
            || status.completion.is_some_and(|result| result.is_err())
        {
            return Err(RegistryError::Unavailable);
        }
        Ok(status.launch == LaunchStatus::Started
            && status.completed_at.is_some()
            && status.completion.is_some_and(|result| result.is_ok()))
    }
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
    pub fn publish(
        mut self,
        ticket: PidRegisteredCommand,
    ) -> Result<RegisteredStart, RegistryError> {
        let identity = ticket.identity();
        if identity.session != self.inner.session || !ticket.belongs_to(self.inner.binding) {
            return Err(RegistryError::StaleIdentity);
        }
        let record = Arc::new(Record {
            identity,
            metadata: self.metadata,
            state: Mutex::new(State::default()),
            retired: Notify::new(),
            _slot: self.slot,
        });
        {
            let mut table = self
                .inner
                .table
                .lock()
                .map_err(|_| RegistryError::Unavailable)?;
            if table.records.contains_key(&identity.pid) {
                return Err(RegistryError::StaleIdentity);
            }
            if let Some(tag) = record.metadata.tag.as_deref() {
                let claim = self.tag_claim.as_mut().ok_or(RegistryError::Unavailable)?;
                if claim.tag.as_deref() != Some(tag) || !table.pending_tags.contains(tag) {
                    return Err(RegistryError::Unavailable);
                }
                if let Some(previous) = tagged(&table, tag)?.cloned() {
                    if !previous.finished()? {
                        return Err(RegistryError::AlreadyExists);
                    }
                    table.records.remove(&previous.identity.pid);
                    // The original timer/handles release their own permits.
                    // Waking retirement does not fabricate capacity or stop a process.
                    previous.retired.notify_one();
                }
                claim.release(&mut table);
            }
            table.records.insert(identity.pid, record.clone());
        }
        let (reply, receive) = oneshot::channel();
        // Created before spawning: even an unpolled/aborted task leaves an
        // unavailable record occupying its original slot, not false credit.
        let guard = Guard {
            inner: self.inner,
            record,
            removed: false,
        };
        task_accounting::spawn(AsyncTask::CommandRegistry, run(ticket, reply, guard));
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
        let mut table = self
            .inner
            .table
            .lock()
            .map_err(|_| RegistryError::Unavailable)?;
        if table
            .records
            .get(&self.record.identity.pid)
            .is_some_and(|actual| Arc::ptr_eq(actual, &self.record))
        {
            table.records.remove(&self.record.identity.pid);
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
        tokio::select! {
            () = tokio::time::sleep_until(expiry) => {}
            () = guard.record.retired.notified() => {}
        }
    }
    let _ = guard.remove();
}

#[cfg(test)]
mod tests;
