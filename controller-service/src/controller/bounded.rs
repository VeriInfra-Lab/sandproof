//! Opt-in physical positions, durably paired with the existing core reducer.
//! No retirement, checkpointing or general maintenance API is enabled here.
use super::{ControllerJournal, storage, transaction::WorldTransaction};
use crate::{AdmissionError, CrashInjector, CrashPoint, JournalError, WorkerTransportError};
use rusqlite::{Connection, Transaction, params};
use sandbox_controller_core::{
    self as core,
    maintenance::{
        MaintenanceInput, MaintenanceLimits,
        slots::{HostBinding, HostSlotId, HostSlotPhase, SlotInput},
    },
};
use serde::{Deserialize, Serialize};

/// Explicit deployment mapping. None of these limits has a production default.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BoundedHostConfig {
    pub maintenance: MaintenanceLimits,
    pub uid_base: u32,
    /// First deployment-allocated /30 index in the finite 100.64.0.0/10 pool.
    pub uplink_base: u64,
    /// Derived from the deployment's explicit data-transfer per-VM cap.
    pub max_data_routes_per_slot: u64,
    pub max_data_routes_global: u64,
    pub data_route_control_reserve: u64,
}

impl BoundedHostConfig {
    /// # Errors
    /// Rejects invalid limits, reserved UIDs and exhausted physical IPv4 space.
    pub fn validate(&self) -> Result<(), JournalError> {
        let slots = self.maintenance.max_host_slots;
        if self.data_route_control_reserve == 0
            || self.max_data_routes_per_slot <= self.data_route_control_reserve
            || self.max_data_routes_global < self.max_data_routes_per_slot
            || usize::try_from(self.max_data_routes_global).is_err()
            || self.max_data_routes_per_slot == 0
            || usize::try_from(self.max_data_routes_per_slot).is_err()
            || slots == 0
            || slots > 1_048_576
            || self
                .uplink_base
                .checked_add(slots - 1)
                .is_none_or(|last| last >= 1_048_576)
            || self.maintenance.max_retained_results == 0
            || usize::try_from(self.maintenance.max_retained_results).is_err()
            || self.uid_base == 0
            || u64::from(self.uid_base)
                .checked_add(slots - 1)
                .is_none_or(|uid| uid >= u64::from(u32::MAX))
        {
            return Err(JournalError::UnsupportedStore);
        }
        Ok(())
    }
}

/// Kept per physical position, including the predecessor needed after a lost
/// host bind acknowledgement. It is never reconstructed from current maxima.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HostSlotContext {
    pub position: HostSlotId,
    pub expected_previous: Option<core::VmId>,
    pub binding: HostBinding,
}

pub(super) fn config(connection: &Connection) -> Result<Option<BoundedHostConfig>, JournalError> {
    let format: i64 = connection.query_row("PRAGMA user_version", [], |row| row.get(0))?;
    if format != crate::journal::BOUNDED_FORMAT_VERSION {
        return Ok(None);
    }
    let bytes: Vec<u8> = connection.query_row(
        "SELECT config_json FROM controller_bounded_config WHERE id=1",
        [],
        |row| row.get(0),
    )?;
    let config: BoundedHostConfig = serde_json::from_slice(&bytes)?;
    config.validate()?;
    Ok(Some(config))
}

pub(super) fn configure(
    tx: &Transaction<'_>,
    world: core::World,
    requested: Option<BoundedHostConfig>,
    now_ms: u64,
) -> Result<core::World, JournalError> {
    if let Some(stored) = config(tx)? {
        if requested.as_ref() != Some(&stored) || world.maintenance_state().is_none() {
            return Err(JournalError::UnsupportedStore);
        }
        let contexts = validate_contexts(tx, &world, &stored)?;
        if contexts != storage::replayed_host_contexts(tx)?
            || world
                .maintenance_state()
                .map(core::maintenance::MaintenanceState::limits)
                != Some(stored.maintenance)
        {
            return Err(JournalError::CorruptStore);
        }
        return Ok(world);
    }
    let Some(requested) = requested else {
        if world.maintenance_state().is_some() {
            return Err(JournalError::CorruptStore);
        }
        return Ok(world);
    };
    requested.validate()?;
    let count: i64 = tx.query_row("SELECT count(*) FROM controller_inputs", [], |row| {
        row.get(0)
    })?;
    if count != 0 || !world.sandbox_records().is_empty() {
        return Err(JournalError::UnsupportedStore);
    }
    tx.execute_batch("CREATE TABLE controller_bounded_config (id INTEGER PRIMARY KEY CHECK(id=1), config_json BLOB NOT NULL); CREATE TABLE controller_host_slots (position INTEGER PRIMARY KEY CHECK(position>=0), context_json BLOB NOT NULL);")?;
    tx.execute(
        "INSERT INTO controller_bounded_config VALUES(1,?1)",
        [serde_json::to_vec(&requested)?],
    )?;
    let decision = storage::append_maintenance(
        tx,
        &world,
        MaintenanceInput::Enable {
            limits: requested.maintenance,
            now_ms,
        },
    )?;
    tx.pragma_update(None, "user_version", crate::journal::BOUNDED_FORMAT_VERSION)?;
    Ok(decision.next)
}

fn contexts(connection: &Connection) -> Result<Vec<HostSlotContext>, JournalError> {
    let mut statement = connection
        .prepare("SELECT position,context_json FROM controller_host_slots ORDER BY position")?;
    let mut contexts = Vec::new();
    for row in statement.query_map([], |row| {
        Ok((row.get::<_, i64>(0)?, row.get::<_, Vec<u8>>(1)?))
    })? {
        let (position, bytes) = row?;
        let context: HostSlotContext = serde_json::from_slice(&bytes)?;
        if u64::try_from(position).ok() != Some(context.position.0) {
            return Err(JournalError::CorruptStore);
        }
        contexts.push(context);
    }
    Ok(contexts)
}

fn validate_contexts(
    connection: &Connection,
    world: &core::World,
    config: &BoundedHostConfig,
) -> Result<Vec<HostSlotContext>, JournalError> {
    let state = world
        .maintenance_state()
        .ok_or(JournalError::CorruptStore)?;
    let contexts = contexts(connection)?;
    if contexts.len() != state.host_slots().len()
        || contexts.len() as u64 > config.maintenance.max_host_slots
    {
        return Err(JournalError::CorruptStore);
    }
    for context in &contexts {
        if context.position.0 >= config.maintenance.max_host_slots
            || context
                .expected_previous
                .is_some_and(|old| old >= context.binding.vm)
            || state
                .host_slot(context.position)
                .is_none_or(|row| row.binding != context.binding)
        {
            return Err(JournalError::CorruptStore);
        }
    }
    Ok(contexts)
}

pub(super) fn reserve_host_slot(
    tx: &WorldTransaction<'_>,
    sandbox: core::SandboxId,
) -> Result<(), AdmissionError> {
    let Some(config) = config(tx)? else {
        return Ok(());
    };
    let world = tx.world();
    validate_contexts(tx, &world, &config)?;
    let state = world
        .maintenance_state()
        .ok_or(JournalError::CorruptStore)?;
    let position = (0..config.maintenance.max_host_slots)
        .map(HostSlotId)
        .find(|position| {
            state
                .host_slot(*position)
                .is_none_or(|row| row.phase == HostSlotPhase::Free)
        })
        .ok_or(AdmissionError::QuotaExceeded)?;
    let record = world
        .sandbox_record(sandbox)
        .ok_or(JournalError::CorruptStore)?;
    let operation = record.current_operation.ok_or(JournalError::CorruptStore)?;
    let plan = world
        .operation_plan(core::OperationKey {
            sandbox,
            operation: operation.id,
        })
        .ok_or(JournalError::CorruptStore)?;
    let context = HostSlotContext {
        position,
        expected_previous: state.host_slot(position).map(|row| row.binding.vm),
        binding: HostBinding {
            owner: record.owner,
            sandbox,
            generation: record.generation,
            vm: plan.vm.ok_or(JournalError::CorruptStore)?,
            network: plan.slot.ok_or(JournalError::CorruptStore)?,
        },
    };
    storage::apply_maintenance(
        tx,
        MaintenanceInput::Slot(SlotInput::Bind {
            position,
            expected_previous: context.expected_previous,
            binding: context.binding,
        }),
    )?;
    tx.execute("INSERT INTO controller_host_slots(position,context_json) VALUES(?1,?2) ON CONFLICT(position) DO UPDATE SET context_json=excluded.context_json", params![i64::try_from(position.0).map_err(|_| JournalError::CorruptStore)?,serde_json::to_vec(&context).map_err(JournalError::Json)?])?;
    Ok(())
}

impl ControllerJournal {
    /// Read-only startup preflight, before enabling a node's bounded marker.
    /// The eventual IMMEDIATE open repeats every check. This is not admission
    /// authority and cannot substitute for independently empty host inventory.
    /// # Errors
    /// Rejects nonempty legacy state, changed configuration or unknown formats.
    pub fn preflight_bounded(
        path: impl AsRef<std::path::Path>,
        namespace: &str,
        requested: &BoundedHostConfig,
    ) -> Result<(), JournalError> {
        requested.validate()?;
        let path = path.as_ref();
        match std::fs::symlink_metadata(path) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(error) => return Err(error.into()),
            Ok(metadata) if !metadata.is_file() || metadata.file_type().is_symlink() => {
                return Err(JournalError::UnsupportedStore);
            }
            Ok(_) => {}
        }
        let connection =
            Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)?;
        connection.execute_batch("PRAGMA trusted_schema=OFF;")?;
        let format: i64 = connection.query_row("PRAGMA user_version", [], |row| row.get(0))?;
        if format == crate::journal::BOUNDED_FORMAT_VERSION {
            if config(&connection)?.as_ref() != Some(requested) {
                return Err(JournalError::UnsupportedStore);
            }
        } else if matches!(format, 1 | 2) {
            let count: i64 =
                connection.query_row("SELECT count(*) FROM controller_inputs", [], |row| {
                    row.get(0)
                })?;
            if count != 0 {
                return Err(JournalError::UnsupportedStore);
            }
        } else {
            return Err(JournalError::UnsupportedStore);
        }
        let stored: String = connection.query_row(
            "SELECT namespace FROM controller_meta WHERE id=1",
            [],
            |row| row.get(0),
        )?;
        if stored != namespace {
            return Err(JournalError::UnsupportedStore);
        }
        Ok(())
    }

    /// # Errors
    /// Rejects changed writers or malformed persisted configuration.
    pub fn bounded_host_config(&self) -> Result<Option<BoundedHostConfig>, JournalError> {
        self.committed_world
            .ensure_ready(&self.journal.connection)?;
        config(&self.journal.connection)
    }

    /// All current/last bindings, one per occupied, releasing or free position.
    /// # Errors
    /// Rejects disagreement with the durable reducer state.
    pub fn host_slot_contexts(&self) -> Result<Vec<HostSlotContext>, JournalError> {
        let world = self.world()?;
        match config(&self.journal.connection)? {
            Some(config) => validate_contexts(&self.journal.connection, &world, &config),
            None => Ok(Vec::new()),
        }
    }

    /// # Errors
    /// Rejects any missing or inconsistent reducer binding.
    pub fn host_slot_inventory(
        &self,
    ) -> Result<Vec<(HostSlotContext, HostSlotPhase)>, JournalError> {
        let world = self.world()?;
        self.host_slot_contexts()?
            .into_iter()
            .map(|context| {
                let phase = world
                    .maintenance_state()
                    .and_then(|state| state.host_slot(context.position))
                    .ok_or(JournalError::CorruptStore)?
                    .phase;
                Ok((context, phase))
            })
            .collect()
    }

    /// # Errors
    /// Rejects corrupt context. A superseded sandbox has no current position.
    pub fn host_slot_context(
        &self,
        sandbox: core::SandboxId,
    ) -> Result<Option<HostSlotContext>, JournalError> {
        Ok(self
            .host_slot_contexts()?
            .into_iter()
            .find(|context| context.binding.sandbox == sandbox))
    }

    /// Build retry anchors and publication references remain conservative pins
    /// until their explicit migration. This is not a caller-supplied certificate.
    /// # Errors
    /// Rejects unreadable durable references.
    pub fn host_slot_release_pinned(&self, sandbox: core::SandboxId) -> Result<bool, JournalError> {
        let options = self.create_options(sandbox).map_err(|error| match error {
            AdmissionError::Journal(error) => error,
            _ => JournalError::CorruptStore,
        })?;
        Ok(options.metadata.contains_key("_sandproof_build_id")
            || self.template_validation_sandboxes()?.contains(&sandbox.0))
    }

    /// Commit the release intent before invoking the trusted exact-host release
    /// operation, then commit its acknowledgement. The caller must hold the
    /// service/host serial boundary and prove all data routes and pins drained.
    /// No host callback runs unless the core accepts the exact terminal binding.
    /// # Errors
    /// Unknown host work keeps Releasing. Unknown commit invalidates the cache.
    pub fn release_host_slot(
        &mut self,
        context: HostSlotContext,
        release: impl FnOnce(&HostSlotContext) -> Result<(), WorkerTransportError>,
        crashes: &mut impl CrashInjector,
    ) -> Result<(), JournalError> {
        if self.host_slot_release_pinned(context.binding.sandbox)? {
            return Err(JournalError::UnresolvedAction);
        }
        self.transition_host_release(context, false, crashes)?;
        release(&context).map_err(|_| JournalError::UnresolvedAction)?;
        crashes.hit(CrashPoint::AfterWorkerCall)?;
        self.transition_host_release(context, true, crashes)
    }

    fn transition_host_release(
        &mut self,
        context: HostSlotContext,
        confirmed: bool,
        crashes: &mut impl CrashInjector,
    ) -> Result<(), JournalError> {
        let tx = WorldTransaction::begin(&mut self.journal.connection, &self.committed_world)?;
        let config = config(&tx)?.ok_or(JournalError::UnsupportedInput)?;
        let world = tx.world();
        if !validate_contexts(&tx, &world, &config)?.contains(&context) {
            return Err(JournalError::Conflict);
        }
        let row = world
            .maintenance_state()
            .and_then(|state| state.host_slot(context.position))
            .ok_or(JournalError::CorruptStore)?;
        if row.phase != HostSlotPhase::Free {
            storage::apply_maintenance(
                &tx,
                MaintenanceInput::Slot(if confirmed {
                    SlotInput::ConfirmRelease {
                        position: context.position,
                        binding: context.binding,
                    }
                } else {
                    SlotInput::BeginRelease {
                        position: context.position,
                        binding: context.binding,
                    }
                }),
            )?;
        }
        crashes.hit(CrashPoint::BeforeCommit)?;
        tx.commit()?;
        crashes.hit(CrashPoint::AfterCommit)
    }
}
