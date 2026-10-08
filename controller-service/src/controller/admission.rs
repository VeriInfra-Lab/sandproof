//! Lease and admission accounting lives beside the reducer's accepted-input
//! transaction. The core still owns lifecycle transitions and authorization.

use super::{ControllerJournal, identity, storage, transaction::WorldTransaction};
use crate::templates::{ComputeLimits, TemplateBinding};
use crate::{CrashInjector, CrashPoint, JournalError};
use rusqlite::{OptionalExtension, Transaction, params};
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
    /// Original network options, when supplied by the create caller.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub networking: Option<CreateNetworking>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    /// Trusted resolved descriptor, never copied from a client's host-path input.
    pub template: Option<TemplateBinding>,
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
    /// Maximum encoded create options, including resolved template paths.
    pub const MAX_ENCODED_BYTES: usize = 64 * 1024;

    /// Check bounded, JSON-safe values before any durable or host effect.
    /// # Errors
    /// Rejects invalid environment names, NULs or oversized input.
    pub fn validate(&self) -> Result<(), AdmissionError> {
        self.encoded().map(|_| ())
    }

    fn encoded(&self) -> Result<Vec<u8>, AdmissionError> {
        if self
            .template
            .as_ref()
            .is_some_and(|template| !template.is_valid())
        {
            return Err(AdmissionError::InvalidCreateOptions);
        }
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
        if encoded.len() > Self::MAX_ENCODED_BYTES {
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
    /// Trusted internal bound for reaching Running, independent of the
    /// client's usable lifetime. Frozen with the accepted create.
    pub startup_timeout_secs: u64,
}

pub const DEFAULT_TIMEOUT_SECS: u64 = 300;
pub const DEFAULT_STARTUP_TIMEOUT_SECS: u64 = 300;

fn reserve_startup_in_admission(
    tx: &WorldTransaction<'_>,
    namespace: &str,
    key: core::OperationKey,
) -> Result<core::ControllerDecision, AdmissionError> {
    // Both reducer transitions must commit with the lease and launch options.
    // Otherwise a crash before reservation can strand an accepted create.
    let reserved = storage::apply(
        tx,
        namespace,
        core::ControllerInput::Scheduler(core::SchedulerInput::ReserveStartup(key)),
    )
    .map_err(|error| match error {
        JournalError::ControllerRejected(core::controller::ControllerReject::Scheduler(
            core::scheduler::SchedulerReject::CounterExhausted,
        )) => AdmissionError::QuotaExceeded,
        other => AdmissionError::Journal(other),
    })?;
    let core::ControllerEvent::Protocol(core::Event::ReserveStartupResources { .. }) =
        reserved.event
    else {
        return Err(AdmissionError::Journal(JournalError::CorruptStore));
    };
    super::bounded::reserve_host_slot(tx, key.sandbox)?;
    Ok(reserved)
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
    /// `None` before successful startup is published.
    pub started_at_ms: Option<i64>,
    pub deadline_ms: i64,
    pub max_deadline_ms: i64,
    /// Immutable admission-time watchdog; never used to expire Running.
    pub startup_deadline_ms: i64,
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
    checked_deadline(now_ms, seconds)
}

fn checked_deadline(now_ms: i64, seconds: u64) -> Result<i64, AdmissionError> {
    if now_ms < 0 {
        return Err(AdmissionError::InvalidClock);
    }
    if seconds == 0 {
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
    let stored: Option<(String, i64, i64, Option<i64>, i64, i64)> = tx
        .query_row(
            "SELECT owner_id, deadline_ms, disk_bytes, started_at_ms, max_deadline_ms, startup_deadline_ms FROM sandbox_lifetimes WHERE sandbox_id=?1",
            [id.as_str()],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?, row.get(5)?)),
        )
        .optional()?;
    stored.map_or(
        Ok(None),
        |(
            stored_owner,
            deadline_ms,
            bytes,
            started_at_ms,
            max_deadline_ms,
            startup_deadline_ms,
        )| {
            if stored_owner != identity::owner_id(owner)?.as_str()
                || deadline_ms < 0
                || startup_deadline_ms < 0
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
                startup_deadline_ms,
                disk_bytes: u64::try_from(bytes).map_err(|_| AdmissionError::CorruptLifetime)?,
            }))
        },
    )
}

fn active(record: &core::SandboxRecord) -> bool {
    record.phase != core::Phase::Destroyed
}

pub(super) fn accounted(
    world: &core::World,
    sandbox: core::SandboxId,
    record: &core::SandboxRecord,
) -> bool {
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
        let tx = WorldTransaction::begin(&mut self.journal.connection, &self.committed_world)?;
        let world = tx.world();
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
        if started_at_ms >= lifetime.startup_deadline_ms {
            // A late-ready VM cannot win a race against its persisted startup
            // watchdog merely because the periodic scanner has not run yet.
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
            core::ControllerInput::Publication(core::PublicationInput::Running(sandbox)),
        )?;
        let id = identity::sandbox_id(&self.namespace, sandbox)?;
        tx.execute(
            "UPDATE sandbox_lifetimes SET started_at_ms=?1, deadline_ms=?2, max_deadline_ms=?3 WHERE sandbox_id=?4",
            params![started_at_ms, end, max_end, id.as_str()],
        )?;
        crashes.hit(CrashPoint::BeforeCommit)?;
        tx.commit()?;
        crashes.hit(CrashPoint::AfterCommit)?;
        Ok(decision.next)
    }

    /// Accept a create and its lease/physical-disk budget in the same `SQLite`
    /// transaction as the reducer input and outbox. An existing active sandbox
    /// without a reservation is corrupt and fails admission closed.
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
        self.admit_with_compute(request, admission, None, None, crashes)
    }

    /// Atomically bind a prebuilt template and charge its actual resource shape.
    /// # Errors
    /// Rejects missing/mismatched descriptors, compute quotas and unmanaged instances.
    pub fn admit_template_create(
        &mut self,
        request: core::CreateRequest,
        admission: CreateAdmission,
        compute: ComputeLimits,
        crashes: &mut impl CrashInjector,
    ) -> Result<core::ControllerDecision, AdmissionError> {
        self.admit_with_compute(request, admission, Some(compute), None, crashes)
    }

    /// The validation VM identity and create intent commit together, before host work.
    /// # Errors
    /// Refuses an already attached, foreign, or changed publication.
    pub fn admit_template_validation(
        &mut self,
        request: core::CreateRequest,
        admission: CreateAdmission,
        compute: ComputeLimits,
        publication: &str,
        crashes: &mut impl CrashInjector,
    ) -> Result<core::ControllerDecision, AdmissionError> {
        self.admit_with_compute(
            request,
            admission,
            Some(compute),
            Some(publication),
            crashes,
        )
    }

    #[expect(
        clippy::too_many_lines,
        reason = "all resource charges and the reducer intent must share one transaction"
    )]
    fn admit_with_compute(
        &mut self,
        request: core::CreateRequest,
        admission: CreateAdmission,
        compute: Option<ComputeLimits>,
        publication: Option<&str>,
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
        self.committed_world
            .ensure_ready(&self.journal.connection)?;
        let cap = super::capacity::config(&self.journal.connection)?
            .ok_or(JournalError::UnsupportedStore)?;
        if limits.cleanup_reserve_bytes < cap.max_online_storage_bytes {
            return Err(JournalError::MaintenanceCapacity.into());
        }
        match (&options.template, compute) {
            (Some(template), Some(limits))
                if limits.is_valid()
                    && template.resources.disk_bytes == disk_bytes
                    && template.owner.is_none_or(|owner| owner == request.owner.0) => {}
            (None, None) => {}
            _ => return Err(AdmissionError::InvalidCreateOptions),
        }
        let end = deadline(now_ms, timeout_secs, limits)?;
        let startup_deadline_ms = checked_deadline(now_ms, limits.startup_timeout_secs)?;
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
        // Persist expiration at the actual admission timestamp, not an earlier
        // HTTP observation. A rejected Admit must never resurrect after clock rollback.
        if options.metadata.contains_key("_sandproof_build_window") {
            self.advance_build_windows(
                u64::try_from(now_ms).map_err(|_| AdmissionError::InvalidClock)?,
                crashes,
            )?;
        }
        let tx = WorldTransaction::begin(&mut self.journal.connection, &self.committed_world)?;
        let schema: i64 = tx.query_row(
            "SELECT version FROM controller_meta WHERE id=1",
            [],
            |row| row.get(0),
        )?;
        // Configuration rollback cannot introduce uncharged instances.
        let private_without_templates = schema >= 11
            && super::private_build::config(&tx)?.is_some()
            && tx.query_row("SELECT count(*) FROM sqlite_master WHERE type='table' AND name IN ('admitted_templates','template_registry_meta')", [], |row| row.get::<_,i64>(0))? == 0;
        if compute.is_none() && schema >= 8 && !private_without_templates {
            return Err(AdmissionError::InvalidCreateOptions);
        }
        let world = tx.world();
        if storage::unresolved(&world, request.sandbox) {
            return Err(JournalError::UnresolvedAction.into());
        }
        let mut global = 0_u64;
        let mut tenant = 0_u64;
        let mut global_disk = 0_u64;
        let mut tenant_disk = 0_u64;
        let mut global_cpu = 0_u64;
        let mut tenant_cpu = 0_u64;
        let mut global_memory = 0_u64;
        let mut tenant_memory = 0_u64;
        for (id, record) in world.sandbox_records() {
            if !accounted(&world, *id, record) {
                continue;
            }
            let lifetime = read_lifetime(&tx, &self.namespace, *id, record.owner)?
                .ok_or(AdmissionError::UnmanagedActive)?;
            if compute.is_some() {
                let bytes: Vec<u8> = tx.query_row(
                    "SELECT options_json FROM sandbox_create_options WHERE sandbox_id=?1",
                    [identity::sandbox_id(&self.namespace, *id)?.as_str()],
                    |row| row.get(0),
                )?;
                let stored: CreateOptions = serde_json::from_slice(&bytes)
                    .map_err(|_| AdmissionError::InvalidCreateOptions)?;
                if stored.encoded()? != bytes {
                    return Err(AdmissionError::InvalidCreateOptions);
                }
                let template = stored.template.ok_or(AdmissionError::UnmanagedActive)?;
                if template.resources.disk_bytes != lifetime.disk_bytes
                    || template.owner.is_some_and(|owner| owner != record.owner.0)
                {
                    return Err(AdmissionError::CorruptLifetime);
                }
                global_cpu = global_cpu
                    .checked_add(u64::from(template.resources.cpu_count))
                    .ok_or(AdmissionError::QuotaExceeded)?;
                global_memory = global_memory
                    .checked_add(template.resources.host_memory_bytes)
                    .ok_or(AdmissionError::QuotaExceeded)?;
                if record.owner == request.owner {
                    tenant_cpu = tenant_cpu
                        .checked_add(u64::from(template.resources.cpu_count))
                        .ok_or(AdmissionError::QuotaExceeded)?;
                    tenant_memory = tenant_memory
                        .checked_add(template.resources.host_memory_bytes)
                        .ok_or(AdmissionError::QuotaExceeded)?;
                }
            }
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
        if let (Some(template), Some(compute)) = (&options.template, compute) {
            let shape = template.resources;
            if global_cpu
                .checked_add(u64::from(shape.cpu_count))
                .is_none_or(|total| total > compute.max_vcpus_global)
                || tenant_cpu
                    .checked_add(u64::from(shape.cpu_count))
                    .is_none_or(|total| total > compute.max_vcpus_per_tenant)
                || global_memory
                    .checked_add(shape.host_memory_bytes)
                    .is_none_or(|total| total > compute.max_memory_bytes_global)
                || tenant_memory
                    .checked_add(shape.host_memory_bytes)
                    .is_none_or(|total| total > compute.max_memory_bytes_per_tenant)
            {
                return Err(AdmissionError::QuotaExceeded);
            }
            // A binary without this feature must not reinterpret these as global base.
            // Template IDs are immutable after their first accepted use, even
            // across catalog replacement. Aliases may point at another ID.
            tx.execute("CREATE TABLE IF NOT EXISTS admitted_templates (template_id TEXT PRIMARY KEY, binding_json BLOB NOT NULL)", [])?;
            let binding =
                serde_json::to_vec(template).map_err(|_| AdmissionError::InvalidCreateOptions)?;
            let previous: Option<Vec<u8>> = tx
                .query_row(
                    "SELECT binding_json FROM admitted_templates WHERE template_id=?1",
                    [&template.template_id],
                    |row| row.get(0),
                )
                .optional()?;
            if previous.as_ref().is_some_and(|stored| *stored != binding) {
                return Err(AdmissionError::InvalidCreateOptions);
            }
            tx.execute("INSERT OR IGNORE INTO admitted_templates (template_id, binding_json) VALUES (?1, ?2)", params![template.template_id, binding])?;
            // Older binaries must refuse journals containing startup defaults;
            // Admitting another base template must never lower this guard.
            tx.execute(
                "UPDATE controller_meta SET version=MAX(version, ?1) WHERE id=1",
                [if template.startup.is_some() { 9 } else { 8 }],
            )?;
        }
        let sandbox = request.sandbox;
        let operation = request.operation;
        let owner = request.owner;
        storage::apply(
            &tx,
            &self.namespace,
            core::ControllerInput::Request(core::Request::Create(request)),
        )?;
        let mut reserved = reserve_startup_in_admission(
            &tx,
            &self.namespace,
            core::OperationKey { sandbox, operation },
        )?;
        tx.execute(
            "INSERT INTO sandbox_lifetimes (sandbox_id, owner_id, deadline_ms, disk_bytes, started_at_ms, max_deadline_ms, startup_deadline_ms) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![identity::sandbox_id(&self.namespace, sandbox)?.as_str(),
                identity::owner_id(owner)?.as_str(), end, i64::try_from(disk_bytes).map_err(|_| AdmissionError::QuotaExceeded)?, now_ms, max_deadline_ms, startup_deadline_ms],
        )?;
        tx.execute(
            "INSERT INTO sandbox_create_options (sandbox_id, options_json) VALUES (?1, ?2)",
            params![
                identity::sandbox_id(&self.namespace, sandbox)?.as_str(),
                encoded_options
            ],
        )?;
        super::private_build::attach(
            &tx,
            &self.namespace,
            owner,
            sandbox,
            &options,
            u64::try_from(now_ms).map_err(|_| AdmissionError::InvalidClock)?,
        )?;
        if let Some(publication) = publication {
            super::registry::attach_validation(
                &tx,
                publication,
                owner.0,
                sandbox,
                options
                    .template
                    .as_ref()
                    .ok_or(AdmissionError::InvalidCreateOptions)?,
            )?;
        }
        reserved.next = tx.world();
        crashes.hit(CrashPoint::BeforeCommit)?;
        tx.commit()?;
        crashes.hit(CrashPoint::AfterCommit)?;
        Ok(reserved)
    }

    /// Read the immutable creation data for host initialization or metadata
    /// projection. Legacy admitted sandboxes had only empty options.
    /// # Errors
    /// Rejects malformed persisted JSON instead of silently dropping it.
    pub fn create_options(
        &self,
        sandbox: core::SandboxId,
    ) -> Result<CreateOptions, AdmissionError> {
        self.committed_world
            .ensure_ready(&self.journal.connection)?;
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
        let tx = WorldTransaction::begin(&mut self.journal.connection, &self.committed_world)?;
        let world = tx.world();
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
        tx.commit()?;
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

    /// Atomically record terminal compensation for creates still unfinished
    /// when their already persisted lifetime expires. This only records an
    /// intent and terminal-fence action; host stop follows normal dispatch.
    /// # Errors
    /// Refuses a corrupt/missing lease or an invalid clock without mutation.
    pub fn abort_due_startups(
        &mut self,
        now_ms: i64,
        crashes: &mut impl CrashInjector,
    ) -> Result<usize, AdmissionError> {
        if now_ms < 0 {
            return Err(AdmissionError::InvalidClock);
        }
        let tx = WorldTransaction::begin(&mut self.journal.connection, &self.committed_world)?;
        let world = tx.world();
        let mut aborted = 0;
        let due: Vec<_> = world
            .sandbox_records()
            .iter()
            .filter_map(|(sandbox, record)| {
                (record.phase == core::Phase::Starting
                    && matches!(record.current_operation, Some(op)
                        if op.kind == core::OperationKind::Create
                            && matches!(op.status,
                                core::OperationStatus::Pending | core::OperationStatus::OutcomeUnknown)))
                .then_some((*sandbox, record.owner))
            })
            .collect();
        let maximum_batch =
            super::capacity::config(&tx)?.map_or(u64::MAX, |cap| cap.max_transaction_inputs);
        for (sandbox, owner) in due {
            if aborted as u64 >= maximum_batch {
                break;
            }
            let lifetime = read_lifetime(&tx, &self.namespace, sandbox, owner)?
                .ok_or(AdmissionError::UnmanagedActive)?;
            if lifetime.startup_deadline_ms > now_ms {
                continue;
            }
            storage::apply(
                &tx,
                &self.namespace,
                core::ControllerInput::Request(core::Request::Compensate(
                    core::CompensationRequest {
                        sandbox,
                        failure: core::FailureKind::DeadlineExceeded,
                        evidence: None,
                    },
                )),
            )?;
            aborted += 1;
        }
        if aborted > 0 {
            crashes.hit(CrashPoint::BeforeCommit)?;
            tx.commit()?;
            crashes.hit(CrashPoint::AfterCommit)?;
        }
        Ok(aborted)
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
        let tx = WorldTransaction::begin(&mut self.journal.connection, &self.committed_world)?;
        let world = tx.world();
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
            core::ControllerInput::Request(core::Request::Kill(core::KillRequest {
                actor: core::Actor::SystemController,
                sandbox,
                operation,
            })),
        )?;
        crashes.hit(CrashPoint::BeforeCommit)?;
        tx.commit()?;
        crashes.hit(CrashPoint::AfterCommit)?;
        Ok(decision)
    }
}
