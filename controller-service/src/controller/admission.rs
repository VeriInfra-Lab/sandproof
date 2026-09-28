//! Lease and admission accounting lives beside the reducer's accepted-input
//! transaction. The core still owns lifecycle transitions and authorization.

use super::{ControllerJournal, identity, storage};
use crate::{CrashInjector, CrashPoint, JournalError};
use rusqlite::{OptionalExtension, Transaction, TransactionBehavior, params};
use sandbox_controller_core as core;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::error::Error;
use std::fmt::{self, Display};

pub const DEFAULT_SANDBOX_TIMEOUT_SECS: u64 = 300;

/// Immutable, user-supplied creation data outside the pure lifecycle model.
/// It is committed with the create intent before any host-side initialization.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CreateOptions {
    pub env_vars: BTreeMap<String, String>,
    pub metadata: BTreeMap<String, String>,
}

impl CreateOptions {
    /// Check bounded, JSON-safe values before any durable or host effect.
    /// # Errors
    /// Rejects invalid environment names, NULs or oversized input.
    pub fn validate(&self) -> Result<(), AdmissionError> {
        self.encoded().map(|_| ())
    }

    fn encoded(&self) -> Result<Vec<u8>, AdmissionError> {
        if self.env_vars.iter().any(|(key, value)| {
            key.is_empty() || key.contains('=') || key.contains('\0') || value.contains('\0')
        }) || self
            .metadata
            .iter()
            .any(|(key, value)| key.is_empty() || key.contains('\0') || value.contains('\0'))
        {
            return Err(AdmissionError::InvalidCreateOptions);
        }
        let encoded = serde_json::to_vec(self).map_err(|_| AdmissionError::InvalidCreateOptions)?;
        if encoded.len() > 64 * 1024 {
            return Err(AdmissionError::InvalidCreateOptions);
        }
        Ok(encoded)
    }
}

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
    /// Immutable total lifetime granted at creation, measured from the
    /// persisted start. It is stored per sandbox so restarts cannot reset it.
    pub max_total_lifetime_secs: u64,
}

/// One admission decision's immutable time, resource budget and launch data.
#[derive(Clone, Debug)]
pub struct CreateAdmission {
    pub now_ms: i64,
    pub timeout_secs: u64,
    pub disk_bytes: u64,
    pub limits: AdmissionLimits,
    pub options: CreateOptions,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Lifetime {
    /// `None` only for records migrated from an older schema.
    pub started_at_ms: Option<i64>,
    pub deadline_ms: i64,
    pub max_end_ms: i64,
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
    InvalidCreateOptions,
    MissingCreateOptions,
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
            Self::InvalidCreateOptions => f.write_str("invalid sandbox creation options"),
            Self::MissingCreateOptions => f.write_str("accepted sandbox has no creation options"),
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

fn add_seconds(now_ms: i64, seconds: u64) -> Result<i64, AdmissionError> {
    if now_ms < 0 {
        return Err(AdmissionError::InvalidClock);
    }
    let milliseconds = seconds
        .checked_mul(1000)
        .ok_or(AdmissionError::TimeOverflow)?;
    now_ms
        .checked_add(i64::try_from(milliseconds).map_err(|_| AdmissionError::TimeOverflow)?)
        .ok_or(AdmissionError::TimeOverflow)
}

fn deadline(now_ms: i64, seconds: u64, limits: AdmissionLimits) -> Result<i64, AdmissionError> {
    if seconds == 0 || seconds > limits.max_timeout_secs {
        return Err(AdmissionError::InvalidTimeout);
    }
    add_seconds(now_ms, seconds)
}

struct StoredLifetimeRow {
    owner: String,
    deadline_ms: i64,
    disk_bytes: i64,
    started_at_ms: Option<i64>,
    max_end_ms: Option<i64>,
}

fn read_lifetime(
    tx: &Transaction<'_>,
    namespace: &str,
    sandbox: core::SandboxId,
    owner: core::TenantId,
) -> Result<Option<Lifetime>, AdmissionError> {
    let id = identity::sandbox_id(namespace, sandbox)?;
    let stored: Option<StoredLifetimeRow> = tx
        .query_row(
            "SELECT owner_id, deadline_ms, disk_bytes, started_at_ms, max_end_ms FROM sandbox_lifetimes WHERE sandbox_id=?1",
            [id.as_str()],
            |row| {
                Ok(StoredLifetimeRow {
                    owner: row.get(0)?,
                    deadline_ms: row.get(1)?,
                    disk_bytes: row.get(2)?,
                    started_at_ms: row.get(3)?,
                    max_end_ms: row.get(4)?,
                })
            },
        )
        .optional()?;
    stored.map_or(Ok(None), |row| {
        if row.owner != identity::owner_id(owner)?.as_str()
            || row.deadline_ms < 0
            || row.disk_bytes <= 0
            || row
                .started_at_ms
                .is_some_and(|time| time < 0 || time > row.deadline_ms)
            || row.max_end_ms.is_none_or(|end| end < row.deadline_ms)
        {
            return Err(AdmissionError::CorruptLifetime);
        }
        Ok(Some(Lifetime {
            started_at_ms: row.started_at_ms,
            deadline_ms: row.deadline_ms,
            max_end_ms: row.max_end_ms.ok_or(AdmissionError::CorruptLifetime)?,
            disk_bytes: u64::try_from(row.disk_bytes)
                .map_err(|_| AdmissionError::CorruptLifetime)?,
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
    /// Publish Running and set its externally visible lifetime origin in the
    /// same transaction. The provisional admission clock reserves capacity;
    /// it is not the guest's usable lifetime start. A crash before this commit
    /// leaves Starting, so recovery can retry this exact publication once.
    /// # Errors
    /// Rejects a stale publication, missing lease, invalid clock or store error.
    pub fn publish_running_at(
        &mut self,
        sandbox: core::SandboxId,
        started_at_ms: i64,
        crashes: &mut impl CrashInjector,
    ) -> Result<core::World, JournalError> {
        if started_at_ms < 0 {
            return Err(JournalError::UnsupportedInput);
        }
        let tx = self
            .journal
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let world = storage::load_world(&tx)?;
        if storage::unresolved(&world, sandbox) {
            return Err(JournalError::UnresolvedAction);
        }
        let record = world
            .sandbox_record(sandbox)
            .ok_or(JournalError::UnsupportedInput)?;
        let lease = read_lifetime(&tx, &self.namespace, sandbox, record.owner)
            .map_err(|_| JournalError::CorruptStore)?
            .ok_or(JournalError::CorruptStore)?;
        let admitted_at = lease.started_at_ms.ok_or(JournalError::CorruptStore)?;
        if started_at_ms < admitted_at {
            return Err(JournalError::UnsupportedInput);
        }
        let initial_ms = lease
            .deadline_ms
            .checked_sub(admitted_at)
            .ok_or(JournalError::CorruptStore)?;
        let total_ms = lease
            .max_end_ms
            .checked_sub(admitted_at)
            .ok_or(JournalError::CorruptStore)?;
        let end = started_at_ms
            .checked_add(initial_ms)
            .ok_or(JournalError::UnsupportedInput)?;
        let max_end = started_at_ms
            .checked_add(total_ms)
            .ok_or(JournalError::UnsupportedInput)?;
        let decision = storage::apply(
            &tx,
            &self.namespace,
            &world,
            core::ControllerInput::Publication(core::PublicationInput::Running(sandbox)),
        )?;
        let id = identity::sandbox_id(&self.namespace, sandbox)?;
        tx.execute(
            "UPDATE sandbox_lifetimes SET started_at_ms=?1, deadline_ms=?2, max_end_ms=?3 WHERE sandbox_id=?4",
            params![started_at_ms, end, max_end, id.as_str()],
        )?;
        crashes.hit(CrashPoint::BeforeCommit)?;
        tx.commit().map_err(JournalError::CommitUncertain)?;
        crashes.hit(CrashPoint::AfterCommit)?;
        Ok(decision.next)
    }

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
        self.admit_create_with_options(
            request,
            CreateAdmission {
                now_ms,
                timeout_secs,
                disk_bytes,
                limits,
                options: CreateOptions::default(),
            },
            crashes,
        )
    }

    /// Atomically bind user initialization data to the accepted create.
    /// A restart must read these exact bytes; it cannot infer them from a
    /// later HTTP retry or mint a different launch configuration.
    /// # Errors
    /// Rejects invalid input, quotas, store failures or a rejected create.
    pub fn admit_create_with_options(
        &mut self,
        request: core::CreateRequest,
        admission: CreateAdmission,
        crashes: &mut impl CrashInjector,
    ) -> Result<core::ControllerDecision, AdmissionError> {
        let CreateAdmission {
            now_ms,
            timeout_secs,
            disk_bytes,
            limits,
            options,
        } = admission;
        let encoded_options = options.encoded()?;
        let end = deadline(now_ms, timeout_secs, limits)?;
        let max_end = add_seconds(now_ms, limits.max_total_lifetime_secs)?;
        if limits.max_total_lifetime_secs == 0 || end > max_end {
            return Err(AdmissionError::InvalidTimeout);
        }
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
            "INSERT INTO sandbox_lifetimes (sandbox_id, owner_id, deadline_ms, disk_bytes, started_at_ms, max_end_ms) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![identity::sandbox_id(&self.namespace, sandbox)?.as_str(),
                identity::owner_id(owner)?.as_str(), end, i64::try_from(disk_bytes).map_err(|_| AdmissionError::QuotaExceeded)?, now_ms, max_end],
        )?;
        tx.execute(
            "INSERT INTO sandbox_create_options (sandbox_id, options_json) VALUES (?1, ?2)",
            params![
                identity::sandbox_id(&self.namespace, sandbox)?.as_str(),
                encoded_options
            ],
        )?;
        crashes.hit(CrashPoint::BeforeCommit)?;
        tx.commit().map_err(JournalError::CommitUncertain)?;
        crashes.hit(CrashPoint::AfterCommit)?;
        Ok(decision)
    }

    /// Read the immutable creation data for host initialization or metadata
    /// projection. Legacy admitted sandboxes had only empty options.
    /// # Errors
    /// Rejects malformed persisted JSON instead of silently dropping it.
    pub fn create_options(
        &self,
        sandbox: core::SandboxId,
    ) -> Result<CreateOptions, AdmissionError> {
        let id = identity::sandbox_id(&self.namespace, sandbox)?;
        let encoded: Option<Vec<u8>> = self
            .journal
            .connection
            .query_row(
                "SELECT options_json FROM sandbox_create_options WHERE sandbox_id=?1",
                [id.as_str()],
                |row| row.get(0),
            )
            .optional()?;
        if let Some(bytes) = encoded {
            let options: CreateOptions =
                serde_json::from_slice(&bytes).map_err(|_| AdmissionError::InvalidCreateOptions)?;
            if options.encoded()? != bytes {
                return Err(AdmissionError::InvalidCreateOptions);
            }
            Ok(options)
        } else {
            let admitted: i64 = self.journal.connection.query_row(
                "SELECT count(*) FROM sandbox_lifetimes WHERE sandbox_id=?1",
                [id.as_str()],
                |row| row.get(0),
            )?;
            if admitted != 0 {
                return Err(AdmissionError::MissingCreateOptions);
            }
            Ok(CreateOptions::default())
        }
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

    /// Connect may only retain or extend the current deadline. An omitted
    /// timeout requests the fixed v2 default of 300 seconds. The requested
    /// interval must fit the per-call limit; the durable total end is a cap.
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
        let requested = timeout_secs.unwrap_or(DEFAULT_SANDBOX_TIMEOUT_SECS);
        let candidate = deadline(now_ms, requested, limits)?;
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
        if now_ms > lifetime.max_end_ms {
            return Err(AdmissionError::InvalidTimeout);
        }
        if lifetime.deadline_ms <= now_ms {
            return Err(AdmissionError::Expired);
        }
        let candidate = candidate.min(lifetime.max_end_ms);
        if candidate > lifetime.deadline_ms {
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
    /// Zero and negative values expire access immediately; the expiry worker
    /// later performs the physical stop. Positive values are capped by the
    /// immutable total-life end rather than rejected as per-call overages.
    /// # Errors
    /// Rejects owner/state/expiry/time errors.
    pub fn set_lifetime(
        &mut self,
        actor: &core::Actor,
        sandbox: core::SandboxId,
        now_ms: i64,
        timeout_secs: i64,
    ) -> Result<Lifetime, AdmissionError> {
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
        if actor != &core::Actor::Tenant(record.owner) {
            return Err(AdmissionError::Unauthorized);
        }
        if record.phase != core::Phase::Running {
            return Err(AdmissionError::NotRunning);
        }
        let mut lifetime = read_lifetime(&tx, &self.namespace, sandbox, record.owner)?
            .ok_or(AdmissionError::MissingLifetime)?;
        if now_ms > lifetime.max_end_ms {
            return Err(AdmissionError::InvalidTimeout);
        }
        if lifetime.deadline_ms <= now_ms {
            return Err(AdmissionError::Expired);
        }
        // A large positive timeout is still a request to use the remaining
        // total lifetime. Cap before arithmetic can overflow the clock range.
        let end = if timeout_secs <= 0 {
            now_ms
        } else {
            let remaining_ms = lifetime.max_end_ms.saturating_sub(now_ms);
            let requested_ms = u64::try_from(timeout_secs)
                .map_err(|_| AdmissionError::InvalidTimeout)?
                .saturating_mul(1000);
            if u128::from(requested_ms) >= u128::try_from(remaining_ms).unwrap_or(0) {
                lifetime.max_end_ms
            } else {
                now_ms
                    .checked_add(
                        i64::try_from(requested_ms).map_err(|_| AdmissionError::TimeOverflow)?,
                    )
                    .ok_or(AdmissionError::TimeOverflow)?
            }
        };
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
