//! Lease and admission accounting lives beside the reducer's accepted-input
//! transaction. The core still owns lifecycle transitions and authorization.

mod lifetime_core;

pub use lifetime_core::Lifetime;

use super::{ControllerJournal, identity, storage, transaction::WorldTransaction};
use crate::templates::ComputeLimits;
use crate::{CrashInjector, CrashPoint, JournalError};
use rusqlite::{Connection, OptionalExtension, Transaction, params};
use sandbox_controller_core as core;

fn map_lifetime_reject(error: lifetime_core::LifetimeReject) -> AdmissionError {
    match error {
        lifetime_core::LifetimeReject::InvalidClock => AdmissionError::InvalidClock,
        lifetime_core::LifetimeReject::NotRunning => AdmissionError::NotRunning,
        lifetime_core::LifetimeReject::Unauthorized => AdmissionError::Unauthorized,
        lifetime_core::LifetimeReject::MissingLifetime => AdmissionError::MissingLifetime,
        lifetime_core::LifetimeReject::Expired => AdmissionError::Expired,
        lifetime_core::LifetimeReject::CorruptLifetime => AdmissionError::CorruptLifetime,
    }
}

fn lifetime_phase(phase: core::Phase) -> lifetime_core::LifetimePhase {
    match phase {
        core::Phase::Starting => lifetime_core::LifetimePhase::Starting,
        core::Phase::Running => lifetime_core::LifetimePhase::Running,
        core::Phase::Pausing => lifetime_core::LifetimePhase::Pausing,
        core::Phase::Paused => lifetime_core::LifetimePhase::Paused,
        core::Phase::Revoking => lifetime_core::LifetimePhase::Revoking,
        core::Phase::Destroyed => lifetime_core::LifetimePhase::Destroyed,
    }
}

pub use crate::host_contract::{
    AdmissionError, AdmissionLimits, CreateOptions, DEFAULT_TIMEOUT_SECS,
};

// The same canonical read is also used inside an already validated original
// read transaction. It does not consult/re-enter the autocommit cache guard.
pub(in crate::controller) fn read_create_options(
    connection: &Connection,
    namespace: &str,
    sandbox: core::SandboxId,
) -> Result<CreateOptions, AdmissionError> {
    let id = identity::sandbox_id(namespace, sandbox)?;
    let encoded: Option<Vec<u8>> = connection
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
        let admitted: i64 = connection.query_row(
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

pub(super) fn read_lifetime(
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
            let expected_owner = identity::owner_id(owner)?;
            lifetime_core::decode_row(
                lifetime_core::RawLifetimeRow {
                    owner: stored_owner.as_bytes(),
                    deadline_ms,
                    bytes,
                    started_at_ms,
                    max_deadline_ms,
                    startup_deadline_ms,
                },
                expected_owner.as_str().as_bytes(),
            )
            .map(Some)
            .map_err(map_lifetime_reject)
        },
    )
}

/// The original update preconditions, shared by local API normalization and
/// authenticated fixed-deadline internal intents. Keep their rejection order.
pub(super) fn lifetime_for_update(
    tx: &WorldTransaction<'_>,
    namespace: &str,
    actor: &core::Actor,
    sandbox: core::SandboxId,
    now_ms: i64,
) -> Result<Lifetime, AdmissionError> {
    if now_ms < 0 {
        return Err(AdmissionError::InvalidClock);
    }
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
    let lifetime = read_lifetime(tx, namespace, sandbox, record.owner)?
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
    Ok(lifetime)
}

/// Write only a legal exact deadline. The caller owns the same transaction as
/// its accepted operation/result; this does not renormalize a replay from now.
pub(super) fn write_lifetime_deadline(
    tx: &WorldTransaction<'_>,
    namespace: &str,
    sandbox: core::SandboxId,
    mut lifetime: Lifetime,
    end_ms: i64,
) -> Result<Lifetime, AdmissionError> {
    if end_ms < 0
        || lifetime.started_at_ms.is_some_and(|start| end_ms < start)
        || end_ms > lifetime.max_deadline_ms
    {
        return Err(AdmissionError::InvalidTimeout);
    }
    let id = identity::sandbox_id(namespace, sandbox)?;
    if tx.execute(
        "UPDATE sandbox_lifetimes SET deadline_ms=?1 WHERE sandbox_id=?2",
        params![end_ms, id.as_str()],
    )? != 1
    {
        return Err(AdmissionError::MissingLifetime);
    }
    lifetime.deadline_ms = end_ms;
    Ok(lifetime)
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
        self.admit_with_compute(request, admission, None, None, None, crashes)
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
        self.admit_with_compute(request, admission, Some(compute), None, None, crashes)
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
            None,
            crashes,
        )
    }

    #[expect(
        clippy::too_many_lines,
        reason = "all resource charges and the reducer intent must share one transaction"
    )]
    pub(super) fn admit_with_compute(
        &mut self,
        request: core::CreateRequest,
        admission: CreateAdmission,
        compute: Option<ComputeLimits>,
        publication: Option<&str>,
        internal: Option<&super::internal_inbox::AdmissionAttachment>,
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
        let (global, tenant) = super::admission_usage::usage(
            &tx,
            &self.namespace,
            &world,
            request.owner,
            compute.is_some(),
        )?;
        let (global_cpu, tenant_cpu) = (global.cpu, tenant.cpu);
        let (global_memory, tenant_memory) = (global.memory_bytes, tenant.memory_bytes);
        let (global_disk, tenant_disk) = (global.disk_bytes, tenant.disk_bytes);
        let (global, tenant) = (global.instances, tenant.instances);
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
            // Distributed admission keeps GLOBAL cache authorization immutable
            // across native tenant retirement. Native options remain the exact
            // LOCAL owner projection used by the original core/worker rules.
            let immutable = if let Some(attachment) = internal {
                let registered = &attachment.registered_template;
                let mut projected = registered.clone();
                if projected.owner.is_some() {
                    projected.owner = Some(request.owner.0);
                }
                // Global artifact paths are metadata only. Execution uses the
                // checked host cache's exact local immutable paths.
                projected.kernel.path.clone_from(&template.kernel.path);
                projected.rootfs.path.clone_from(&template.rootfs.path);
                if !registered.is_valid()
                    || registered.owner.is_some_and(|owner| {
                        owner.to_string() != attachment.request.identity.tenant_id.as_str()
                    })
                    || projected != *template
                    || attachment.local.owner != request.owner
                    || attachment.local.sandbox != request.sandbox
                    || attachment.local.operation != request.operation
                {
                    return Err(AdmissionError::InvalidCreateOptions);
                }
                registered
            } else {
                template
            };
            let binding =
                serde_json::to_vec(immutable).map_err(|_| AdmissionError::InvalidCreateOptions)?;
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
        if let Some(attachment) = internal {
            super::internal_inbox::attach_admission(&tx, &self.namespace, attachment)?;
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
        read_create_options(&self.journal.connection, &self.namespace, sandbox)
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
        lifetime_core::check_clock(now_ms).map_err(map_lifetime_reject)?;
        let world = self.world()?;
        let actor_tenant = match actor {
            core::Actor::Tenant(tenant) => Some(tenant.0),
            core::Actor::SystemController => None,
        };
        let owner = lifetime_core::check_record(
            actor_tenant,
            world
                .sandbox_record(sandbox)
                .map(|record| (record.owner.0, lifetime_phase(record.phase))),
        )
        .map_err(map_lifetime_reject)?;
        let tx = self.journal.connection.unchecked_transaction()?;
        lifetime_core::check_live(
            now_ms,
            read_lifetime(&tx, &self.namespace, sandbox, core::TenantId(owner))?,
        )
        .map_err(map_lifetime_reject)
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
        let mut lifetime = lifetime_for_update(&tx, &self.namespace, actor, sandbox, now_ms)?;
        // Widen before multiplying: even i64::MAX seconds clamps correctly.
        let capped =
            (i128::from(now_ms) + seconds * 1000).min(i128::from(lifetime.max_deadline_ms));
        let end = i64::try_from(capped).map_err(|_| AdmissionError::TimeOverflow)?;
        if allow_shorter || end > lifetime.deadline_ms {
            lifetime = write_lifetime_deadline(&tx, &self.namespace, sandbox, lifetime, end)?;
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
