//! Required physical positions, durably paired with the existing core reducer.
//! This module owns binding/release; the checkpoint adapter
//! owns historical retirement. There is no public raw maintenance input API.
use super::{ControllerJournal, storage, transaction::WorldTransaction};
use crate::{AdmissionError, CrashInjector, CrashPoint, JournalError, WorkerTransportError};
use rusqlite::{Connection, Transaction, params};
use sandbox_controller_core::{
    self as core,
    maintenance::{
        MaintenanceInput,
        slots::{HostBinding, HostSlotId, HostSlotPhase, SlotInput},
    },
};

pub use crate::host_contract::{BoundedHostConfig, ConsolePolicy, HostSlotContext};

pub(super) fn config(connection: &Connection) -> Result<Option<BoundedHostConfig>, JournalError> {
    let format: i64 = connection.query_row("PRAGMA user_version", [], |row| row.get(0))?;
    if !matches!(format, crate::journal::MAINTAINED_FORMAT_VERSION) {
        return Err(JournalError::UnsupportedStore);
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

pub(super) fn initialize(
    tx: &Transaction<'_>,
    requested: BoundedHostConfig,
    now_ms: u64,
    max_input_bytes: u64,
) -> Result<core::World, JournalError> {
    requested.validate()?;
    tx.execute_batch("CREATE TABLE controller_bounded_config(id INTEGER PRIMARY KEY CHECK(id=1),config_json BLOB NOT NULL); CREATE TABLE controller_host_slots(position INTEGER PRIMARY KEY CHECK(position>=0),context_json BLOB NOT NULL);")?;
    tx.execute(
        "INSERT INTO controller_bounded_config VALUES(1,?1)",
        [serde_json::to_vec(&requested)?],
    )?;
    let input = core::ControllerInput::Maintenance(MaintenanceInput::Enable {
        limits: requested.maintenance,
        now_ms,
    });
    let world = core::handle_controller(&core::initial_world(), input.clone())
        .map_err(JournalError::ControllerRejected)?
        .next;
    let bytes = super::checkpoint::encode_limited(
        &super::storage::StoredInput { version: 1, input },
        max_input_bytes,
    )?;
    let sequence = super::allocators::next_sql(tx, super::allocators::SqlId::Input)?;
    tx.execute(
        "INSERT INTO controller_inputs(sequence,input_json) VALUES(?1,?2)",
        params![sequence, bytes],
    )?;
    Ok(world)
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

pub(super) fn validate_contexts(
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
    let config = config(tx)?.ok_or(JournalError::UnsupportedStore)?;
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
    /// Immutable deployment identity used to select a shared physical pool.
    #[must_use]
    pub fn deployment_namespace(&self) -> &str {
        &self.namespace
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
        let config = config(&self.journal.connection)?.ok_or(JournalError::UnsupportedStore)?;
        validate_contexts(&self.journal.connection, &world, &config)
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

    /// Unsettled build retry anchors and publication references are conservative
    /// pins. This is not a caller-supplied certificate.
    /// # Errors
    /// Rejects unreadable durable references.
    pub fn host_slot_release_pinned(&self, sandbox: core::SandboxId) -> Result<bool, JournalError> {
        let options = self.create_options(sandbox).map_err(|error| match error {
            AdmissionError::Journal(error) => error,
            _ => JournalError::CorruptStore,
        })?;
        Ok(super::private_build::pinned(
            &self.journal.connection,
            &self.world()?,
            &self.namespace,
            sandbox,
            &options,
        )? || self
            .active_template_validation_sandboxes()?
            .contains(&sandbox.0))
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
        if confirmed {
            super::checkpoint::record_release_witness(
                &tx,
                context,
                row.phase != HostSlotPhase::Free,
            )?;
        }
        crashes.hit(CrashPoint::BeforeCommit)?;
        tx.commit()?;
        crashes.hit(CrashPoint::AfterCommit)
    }
}
