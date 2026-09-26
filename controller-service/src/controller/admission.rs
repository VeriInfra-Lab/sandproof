//! Lease and admission accounting lives beside the reducer's accepted-input
//! transaction. The core still owns lifecycle transitions and authorization.

use super::{ControllerJournal, identity, storage};
use crate::{CrashInjector, CrashPoint, JournalError};
use rusqlite::{OptionalExtension, Transaction, TransactionBehavior, params};
use sandbox_controller_core as core;
use std::error::Error;
use std::fmt::{self, Display};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AdmissionLimits {
    pub max_active_global: u64,
    pub max_active_per_tenant: u64,
    pub max_disk_bytes_global: u64,
    pub max_disk_bytes_per_tenant: u64,
    /// Never assign this much logical host disk to new sandboxes. SQLite/WAL,
    /// process records and cleanup must remain possible at the admission edge.
    pub cleanup_reserve_bytes: u64,
    pub max_timeout_secs: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Lifetime {
    pub deadline_ms: i64,
    pub disk_bytes: u64,
}

#[derive(Debug)]
pub enum AdmissionError {
    Journal(JournalError),
    InvalidTimeout,
    InvalidClock,
    TimeOverflow,
    QuotaExceeded,
    Unauthorized,
    NotRunning,
    Expired,
    NotDue,
    MissingLifetime,
    UnmanagedActive,
    CorruptLifetime,
}

impl Display for AdmissionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Journal(error) => write!(f, "controller: {error}"),
            Self::InvalidTimeout => f.write_str("invalid sandbox lifetime"),
            Self::InvalidClock => f.write_str("invalid service clock"),
            Self::TimeOverflow => f.write_str("lifetime arithmetic overflow"),
            Self::QuotaExceeded => f.write_str("sandbox admission quota exceeded"),
            Self::Unauthorized => f.write_str("caller does not own sandbox"),
            Self::NotRunning => f.write_str("sandbox is not running"),
            Self::Expired => f.write_str("sandbox lifetime has expired"),
            Self::NotDue => f.write_str("sandbox lifetime has not expired"),
            Self::MissingLifetime => f.write_str("sandbox has no registered lifetime"),
            Self::UnmanagedActive => f.write_str("active sandbox has no quota reservation"),
            Self::CorruptLifetime => f.write_str("sandbox lifetime record mismatch"),
        }
    }
}

impl Error for AdmissionError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Journal(error) => Some(error),
            _ => None,
        }
    }
}

impl From<JournalError> for AdmissionError {
    fn from(error: JournalError) -> Self {
        Self::Journal(error)
    }
}

impl From<rusqlite::Error> for AdmissionError {
    fn from(error: rusqlite::Error) -> Self {
        Self::Journal(JournalError::Sqlite(error))
    }
}

fn deadline(now_ms: i64, seconds: u64, limits: AdmissionLimits) -> Result<i64, AdmissionError> {
    if now_ms < 0 {
        return Err(AdmissionError::InvalidClock);
    }
    if seconds == 0 || seconds > limits.max_timeout_secs {
        return Err(AdmissionError::InvalidTimeout);
    }
    let milliseconds = seconds
        .checked_mul(1000)
        .ok_or(AdmissionError::TimeOverflow)?;
    now_ms
        .checked_add(i64::try_from(milliseconds).map_err(|_| AdmissionError::TimeOverflow)?)
        .ok_or(AdmissionError::TimeOverflow)
}

fn read_lifetime(
    tx: &Transaction<'_>,
    namespace: &str,
    sandbox: core::SandboxId,
    owner: core::TenantId,
) -> Result<Option<Lifetime>, AdmissionError> {
    let id = identity::sandbox_id(namespace, sandbox)?;
    let stored: Option<(String, i64, i64)> = tx
        .query_row(
            "SELECT owner_id, deadline_ms, disk_bytes FROM sandbox_lifetimes WHERE sandbox_id=?1",
            [id.as_str()],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .optional()?;
    stored.map_or(Ok(None), |(stored_owner, deadline_ms, bytes)| {
        if stored_owner != identity::owner_id(owner)?.as_str() || deadline_ms < 0 || bytes <= 0 {
            return Err(AdmissionError::CorruptLifetime);
        }
        Ok(Some(Lifetime {
            deadline_ms,
            disk_bytes: u64::try_from(bytes).map_err(|_| AdmissionError::CorruptLifetime)?,
        }))
    })
}

fn active(record: &core::SandboxRecord) -> bool {
    record.phase != core::Phase::Destroyed
}

fn accounted(world: &core::World, sandbox: core::SandboxId, record: &core::SandboxRecord) -> bool {
    if active(record) {
        return true;
    }
    let resources = world.resource_index(sandbox);
    resources.vms.iter().any(|vm| {
        world
            .vm_record(*vm)
            .is_none_or(|value| value.status != core::VmStatus::Reclaimed)
    }) || resources.slots.iter().any(|slot| {
        world
            .slot_record(*slot)
            .is_none_or(|value| !value.reclaimed)
    })
}

impl ControllerJournal {
    /// Accept a create and its lease/physical-disk budget in the same `SQLite`
    /// transaction as the reducer input and outbox. An existing active sandbox
    /// without a reservation fails admission closed during v1 migration.
    /// # Errors
    /// Rejects invalid time, quotas, unmanaged state and core/store failures.
    pub fn admit_create(
        &mut self,
        request: core::CreateRequest,
        now_ms: i64,
        timeout_secs: u64,
        disk_bytes: u64,
        limits: AdmissionLimits,
        crashes: &mut impl CrashInjector,
    ) -> Result<core::ControllerDecision, AdmissionError> {
        let end = deadline(now_ms, timeout_secs, limits)?;
        if disk_bytes == 0
            || disk_bytes > i64::MAX as u64
            || request.actor != core::Actor::Tenant(request.owner)
        {
            return Err(AdmissionError::Unauthorized);
        }
        let tx = self
            .journal
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let world = storage::load_world(&tx)?;
        if storage::unresolved(&world, request.sandbox) {
            return Err(JournalError::UnresolvedAction.into());
        }
        let mut global = 0_u64;
        let mut tenant = 0_u64;
        let mut global_disk = 0_u64;
        let mut tenant_disk = 0_u64;
        for (id, record) in world.sandbox_records() {
            if !accounted(&world, *id, record) {
                continue;
            }
            let lifetime = read_lifetime(&tx, &self.namespace, *id, record.owner)?
                .ok_or(AdmissionError::UnmanagedActive)?;
            global = global.checked_add(1).ok_or(AdmissionError::QuotaExceeded)?;
            global_disk = global_disk
                .checked_add(lifetime.disk_bytes)
                .ok_or(AdmissionError::QuotaExceeded)?;
            if record.owner == request.owner {
                tenant = tenant.checked_add(1).ok_or(AdmissionError::QuotaExceeded)?;
                tenant_disk = tenant_disk
                    .checked_add(lifetime.disk_bytes)
                    .ok_or(AdmissionError::QuotaExceeded)?;
            }
        }
        if global >= limits.max_active_global
            || tenant >= limits.max_active_per_tenant
            || global_disk
                .checked_add(disk_bytes)
                .and_then(|value| value.checked_add(limits.cleanup_reserve_bytes))
                .is_none_or(|value| value > limits.max_disk_bytes_global)
            || tenant_disk
                .checked_add(disk_bytes)
                .is_none_or(|value| value > limits.max_disk_bytes_per_tenant)
        {
            return Err(AdmissionError::QuotaExceeded);
        }
        let sandbox = request.sandbox;
        let owner = request.owner;
        let decision = storage::apply(
            &tx,
            &self.namespace,
            &world,
            core::ControllerInput::Request(core::Request::Create(request)),
        )?;
        tx.execute(
            "INSERT INTO sandbox_lifetimes (sandbox_id, owner_id, deadline_ms, disk_bytes) VALUES (?1, ?2, ?3, ?4)",
            params![identity::sandbox_id(&self.namespace, sandbox)?.as_str(),
                identity::owner_id(owner)?.as_str(), end, i64::try_from(disk_bytes).map_err(|_| AdmissionError::QuotaExceeded)?],
        )?;
        crashes.hit(CrashPoint::BeforeCommit)?;
        tx.commit().map_err(JournalError::CommitUncertain)?;
        crashes.hit(CrashPoint::AfterCommit)?;
        Ok(decision)
    }

    /// Verify owner, Running authority and unexpired lifetime. API data-plane
    /// callers must additionally invoke the exact core exec/file check.
    /// # Errors
    /// Rejects missing, foreign, expired and non-Running sandboxes.
    pub fn live_lifetime(
        &self,
        actor: &core::Actor,
        sandbox: core::SandboxId,
        now_ms: i64,
    ) -> Result<Lifetime, AdmissionError> {
        if now_ms < 0 {
            return Err(AdmissionError::InvalidClock);
        }
        let world = self.world()?;
        let record = world
            .sandbox_record(sandbox)
            .ok_or(AdmissionError::NotRunning)?;
        if actor != &core::Actor::Tenant(record.owner) {
            return Err(AdmissionError::Unauthorized);
        }
        if record.phase != core::Phase::Running {
            return Err(AdmissionError::NotRunning);
        }
        let tx = self.journal.connection.unchecked_transaction()?;
        let lifetime = read_lifetime(&tx, &self.namespace, sandbox, record.owner)?
            .ok_or(AdmissionError::MissingLifetime)?;
        if lifetime.deadline_ms <= now_ms {
            return Err(AdmissionError::Expired);
        }
        Ok(lifetime)
    }

    /// Connect may only retain or extend the current deadline. Omitted timeout
    /// leaves the deadline unchanged; explicit zero is rejected, not omitted.
    /// # Errors
    /// Rejects owner/state/expiry/time errors.
    pub fn connect_lifetime(
        &mut self,
        actor: &core::Actor,
        sandbox: core::SandboxId,
        now_ms: i64,
        timeout_secs: Option<u64>,
        limits: AdmissionLimits,
    ) -> Result<Lifetime, AdmissionError> {
        let candidate = timeout_secs
            .map(|seconds| deadline(now_ms, seconds, limits))
            .transpose()?;
        let tx = self
            .journal
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let world = storage::load_world(&tx)?;
        let record = world
            .sandbox_record(sandbox)
            .ok_or(AdmissionError::NotRunning)?;
        if actor != &core::Actor::Tenant(record.owner) {
            return Err(AdmissionError::Unauthorized);
        }
        if record.phase != core::Phase::Running {
            return Err(AdmissionError::NotRunning);
        }
        let mut lifetime = read_lifetime(&tx, &self.namespace, sandbox, record.owner)?
            .ok_or(AdmissionError::MissingLifetime)?;
        if lifetime.deadline_ms <= now_ms {
            return Err(AdmissionError::Expired);
        }
        if let Some(candidate) = candidate.filter(|candidate| *candidate > lifetime.deadline_ms) {
            let id = identity::sandbox_id(&self.namespace, sandbox)?;
            tx.execute(
                "UPDATE sandbox_lifetimes SET deadline_ms=?1 WHERE sandbox_id=?2",
                params![candidate, id.as_str()],
            )?;
            lifetime.deadline_ms = candidate;
        }
        tx.commit().map_err(JournalError::CommitUncertain)?;
        Ok(lifetime)
    }

    /// Set timeout may shorten or extend the deadline from the server clock.
    /// # Errors
    /// Rejects owner/state/expiry/time errors.
    pub fn set_lifetime(
        &mut self,
        actor: &core::Actor,
        sandbox: core::SandboxId,
        now_ms: i64,
        timeout_secs: u64,
        limits: AdmissionLimits,
    ) -> Result<Lifetime, AdmissionError> {
        let end = deadline(now_ms, timeout_secs, limits)?;
        let tx = self
            .journal
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let world = storage::load_world(&tx)?;
        let record = world
            .sandbox_record(sandbox)
            .ok_or(AdmissionError::NotRunning)?;
        if actor != &core::Actor::Tenant(record.owner) {
            return Err(AdmissionError::Unauthorized);
        }
        if record.phase != core::Phase::Running {
            return Err(AdmissionError::NotRunning);
        }
        let mut lifetime = read_lifetime(&tx, &self.namespace, sandbox, record.owner)?
            .ok_or(AdmissionError::MissingLifetime)?;
        if lifetime.deadline_ms <= now_ms {
            return Err(AdmissionError::Expired);
        }
        let id = identity::sandbox_id(&self.namespace, sandbox)?;
        tx.execute(
            "UPDATE sandbox_lifetimes SET deadline_ms=?1 WHERE sandbox_id=?2",
            params![end, id.as_str()],
        )?;
        lifetime.deadline_ms = end;
        tx.commit().map_err(JournalError::CommitUncertain)?;
        Ok(lifetime)
    }

    /// Return due Running sandboxes for the caller's expiry worker. This is a
    /// read-only hint; `accept_expiry` checks the deadline again atomically.
    /// # Errors
    /// Fails closed if an active sandbox lacks a lease.
    pub fn due_lifetimes(&self, now_ms: i64) -> Result<Vec<core::SandboxId>, AdmissionError> {
        if now_ms < 0 {
            return Err(AdmissionError::InvalidClock);
        }
        let world = self.world()?;
        let tx = self.journal.connection.unchecked_transaction()?;
        let mut due = Vec::new();
        for (id, record) in world.sandbox_records() {
            if !active(record) {
                continue;
            }
            let lifetime = read_lifetime(&tx, &self.namespace, *id, record.owner)?
                .ok_or(AdmissionError::UnmanagedActive)?;
            if record.phase == core::Phase::Running && lifetime.deadline_ms <= now_ms {
                due.push(*id);
            }
        }
        Ok(due)
    }

    /// Atomically recheck expiry and accept the core Kill intent. The returned
    /// outbox still requires the normal trusted host stop/cleanup workflow.
    /// # Errors
    /// An old timer cannot kill a renewed sandbox, and concurrent expiry
    /// cannot create a second kill operation.
    pub fn accept_expiry(
        &mut self,
        sandbox: core::SandboxId,
        operation: core::OperationId,
        now_ms: i64,
        crashes: &mut impl CrashInjector,
    ) -> Result<core::ControllerDecision, AdmissionError> {
        if now_ms < 0 {
            return Err(AdmissionError::InvalidClock);
        }
        let tx = self
            .journal
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let world = storage::load_world(&tx)?;
        let record = world
            .sandbox_record(sandbox)
            .ok_or(AdmissionError::NotRunning)?;
        if record.phase != core::Phase::Running {
            return Err(AdmissionError::NotRunning);
        }
        let lifetime = read_lifetime(&tx, &self.namespace, sandbox, record.owner)?
            .ok_or(AdmissionError::MissingLifetime)?;
        if lifetime.deadline_ms > now_ms {
            return Err(AdmissionError::NotDue);
        }
        if storage::unresolved(&world, sandbox) {
            return Err(JournalError::UnresolvedAction.into());
        }
        let decision = storage::apply(
            &tx,
            &self.namespace,
            &world,
            core::ControllerInput::Request(core::Request::Kill(core::KillRequest {
                actor: core::Actor::SystemController,
                sandbox,
                operation,
            })),
        )?;
        crashes.hit(CrashPoint::BeforeCommit)?;
        tx.commit().map_err(JournalError::CommitUncertain)?;
        crashes.hit(CrashPoint::AfterCommit)?;
        Ok(decision)
    }
}
