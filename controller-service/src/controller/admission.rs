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

/// Immutable, user-supplied creation data outside the pure lifecycle model.
/// It is committed with the create intent before any host-side initialization.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CreateOptions {
    pub env_vars: BTreeMap<String, String>,
    pub metadata: BTreeMap<String, String>,
    /// None for legacy records whose original request cannot be recovered.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub networking: Option<CreateNetworking>,
}

/// Original supported network fields, before canonicalizing the enforced policy.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CreateNetworking {
    pub allow_internet_access: Option<bool>,
    pub network: Option<CreateNetwork>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct CreateNetwork {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub allow_public_traffic: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub allow_out: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub deny_out: Option<Vec<String>>,
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
    /// Frozen into an absolute deadline when an instance is admitted.
    pub max_lifetime_secs: u64,
}

pub const DEFAULT_TIMEOUT_SECS: u64 = 300;

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
    pub max_deadline_ms: i64,
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
    MaxLifetimeExceeded,
    ChangingState,
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
            Self::MaxLifetimeExceeded => f.write_str("maximum sandbox lifetime exceeded"),
            Self::ChangingState => f.write_str("sandbox is changing state"),
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
    let stored: Option<(String, i64, i64, Option<i64>, i64)> = tx
        .query_row(
            "SELECT owner_id, deadline_ms, disk_bytes, started_at_ms, max_deadline_ms FROM sandbox_lifetimes WHERE sandbox_id=?1",
            [id.as_str()],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?)),
        )
        .optional()?;
    stored.map_or(
        Ok(None),
        |(stored_owner, deadline_ms, bytes, started_at_ms, max_deadline_ms)| {
            if stored_owner != identity::owner_id(owner)?.as_str()
                || deadline_ms < 0
                || bytes <= 0
                || deadline_ms > max_deadline_ms
                || started_at_ms.is_some_and(|time| time < 0 || time > deadline_ms)
            {
                return Err(AdmissionError::CorruptLifetime);
            }
            Ok(Some(Lifetime {
                started_at_ms,
                deadline_ms,
                max_deadline_ms,
                disk_bytes: u64::try_from(bytes).map_err(|_| AdmissionError::CorruptLifetime)?,
            }))
        },
    )
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
    /// Bind the visible lifetime origin to the first Running publication.
    /// Admission reserves time and disk before host work, but E2B starts the
    /// usable lifetime after the VM is ready. Both changes commit together;
    /// a pre-commit crash leaves Starting for exact-operation recovery.
    /// # Errors
    /// Rejects stale publication, absent lease, invalid clock or store errors.
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
        let lifetime = read_lifetime(&tx, &self.namespace, sandbox, record.owner)
            .map_err(|_| JournalError::CorruptStore)?
            .ok_or(JournalError::CorruptStore)?;
        let admitted_at = lifetime.started_at_ms.ok_or(JournalError::CorruptStore)?;
        if started_at_ms < admitted_at {
            return Err(JournalError::UnsupportedInput);
        }
        let initial_ms = lifetime
            .deadline_ms
            .checked_sub(admitted_at)
            .ok_or(JournalError::CorruptStore)?;
        let total_ms = lifetime
            .max_deadline_ms
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
            "UPDATE sandbox_lifetimes SET started_at_ms=?1, deadline_ms=?2, max_deadline_ms=?3 WHERE sandbox_id=?4",
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
        if timeout_secs > limits.max_lifetime_secs {
            return Err(AdmissionError::InvalidTimeout);
        }
        let max_deadline_ms = deadline(
            now_ms,
            limits.max_lifetime_secs,
            AdmissionLimits {
                max_timeout_secs: limits.max_lifetime_secs,
                ..limits
            },
        )?;
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
            "INSERT INTO sandbox_lifetimes (sandbox_id, owner_id, deadline_ms, disk_bytes, started_at_ms, max_deadline_ms) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![identity::sandbox_id(&self.namespace, sandbox)?.as_str(),
                identity::owner_id(owner)?.as_str(), end, i64::try_from(disk_bytes).map_err(|_| AdmissionError::QuotaExceeded)?, now_ms, max_deadline_ms],
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

    /// Connect extends by the requested interval (default 300 seconds), capped
    /// at the original instance limit. It never shortens the existing deadline.
    /// # Errors
    /// Rejects invalid requests and owner/state/expiry/time errors.
    pub fn connect_lifetime(
        &mut self,
        actor: &core::Actor,
        sandbox: core::SandboxId,
        now_ms: i64,
        timeout_secs: Option<u64>,
        limits: AdmissionLimits,
    ) -> Result<Lifetime, AdmissionError> {
        let seconds = timeout_secs.unwrap_or(DEFAULT_TIMEOUT_SECS);
        if seconds == 0 || seconds > limits.max_timeout_secs {
            return Err(AdmissionError::InvalidTimeout);
        }
        self.update_lifetime(actor, sandbox, now_ms, i128::from(seconds), false)
    }

    /// Reset from the service clock; nonpositive values expire immediately.
    /// Oversized requests are capped at the immutable total lifetime, not
    /// rejected as a create/connect duration would be.
    /// # Errors
    /// Rejects owner/state/expiry/time errors.
    pub fn set_lifetime(
        &mut self,
        actor: &core::Actor,
        sandbox: core::SandboxId,
        now_ms: i64,
        timeout_secs: i64,
    ) -> Result<Lifetime, AdmissionError> {
        self.update_lifetime(
            actor,
            sandbox,
            now_ms,
            i128::from(timeout_secs.max(0)),
            true,
        )
    }

    fn update_lifetime(
        &mut self,
        actor: &core::Actor,
        sandbox: core::SandboxId,
        now_ms: i64,
        seconds: i128,
        allow_shorter: bool,
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
        if record.phase == core::Phase::Revoking {
            return Err(AdmissionError::ChangingState);
        }
        if record.phase != core::Phase::Running {
            return Err(AdmissionError::NotRunning);
        }
        let mut lifetime = read_lifetime(&tx, &self.namespace, sandbox, record.owner)?
            .ok_or(AdmissionError::MissingLifetime)?;
        if lifetime.started_at_ms.is_some_and(|start| now_ms < start) {
            return Err(AdmissionError::InvalidClock);
        }
        if now_ms > lifetime.max_deadline_ms {
            return Err(AdmissionError::MaxLifetimeExceeded);
        }
        if lifetime.deadline_ms <= now_ms {
            return Err(AdmissionError::Expired);
        }
        // Widen before multiplying: even i64::MAX seconds clamps correctly.
        let capped =
            (i128::from(now_ms) + seconds * 1000).min(i128::from(lifetime.max_deadline_ms));
        let end = i64::try_from(capped).map_err(|_| AdmissionError::TimeOverflow)?;
        if allow_shorter || end > lifetime.deadline_ms {
            let id = identity::sandbox_id(&self.namespace, sandbox)?;
            tx.execute(
                "UPDATE sandbox_lifetimes SET deadline_ms=?1 WHERE sandbox_id=?2",
                params![end, id.as_str()],
            )?;
            lifetime.deadline_ms = end;
        }
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
