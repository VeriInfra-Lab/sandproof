use super::{
    ControllerJournal, HostReleaseWitness, HostSlotContext, WorldTransaction, allocators, config,
    load_world, namespace, projections, sql, storage,
};
use crate::WorkerTransportError;
use crate::{CrashInjector, CrashPoint, JournalError};
use rusqlite::{Connection, OptionalExtension, Transaction, params};
use sandbox_controller_core as core;
use sandbox_controller_core::maintenance::{
    MaintenanceInput,
    retirement::{QuiescenceCertificate, QuiescenceStatus, RetireRequest},
    slots::HostBinding,
};
use std::collections::BTreeMap;

/// A current exact identity plus durable successful release witness. Callers
/// may inspect it, but retirement reconstructs and verifies it transactionally.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RetirementCandidate {
    pub sandbox: core::SandboxId,
    pub owner: core::TenantId,
    pub epoch: core::Epoch,
    pub operation: core::OperationId,
    pub workspace: core::FsId,
    pub release: Option<HostReleaseWitness>,
}

pub(in crate::controller) fn original_binding(
    world: &core::World,
    sandbox: core::SandboxId,
) -> Result<Option<HostBinding>, JournalError> {
    let row = world
        .sandbox_record(sandbox)
        .ok_or(JournalError::CorruptStore)?;
    let mut binding = None;
    for (key, operation) in world.operation_records() {
        if key.sandbox != sandbox || operation.operation.kind != core::OperationKind::Create {
            continue;
        }
        let plan = world
            .operation_plan(*key)
            .ok_or(JournalError::CorruptStore)?;
        match (plan.vm, plan.slot) {
            (Some(vm), Some(network)) => {
                if binding.is_some() {
                    return Err(JournalError::CorruptStore);
                }
                binding = Some(HostBinding {
                    owner: row.owner,
                    sandbox,
                    generation: plan.generation,
                    vm,
                    network,
                });
            }
            (None, None) => {}
            _ => return Err(JournalError::CorruptStore),
        }
    }
    Ok(binding)
}

pub(super) fn store_witness(
    tx: &Transaction<'_>,
    witness: HostReleaseWitness,
) -> Result<(), JournalError> {
    let sandbox =
        super::super::identity::sandbox_id(&namespace(tx)?, witness.context.binding.sandbox)?;
    tx.execute(
        "INSERT INTO controller_release_witnesses(sandbox_id,witness_json) VALUES(?1,?2)",
        params![sandbox.as_str(), serde_json::to_vec(&witness)?],
    )?;
    Ok(())
}

pub(in crate::controller) fn witnesses(
    connection: &Connection,
    world: &core::World,
) -> Result<Vec<HostReleaseWitness>, JournalError> {
    let namespace = namespace(connection)?;
    let mut statement =
        connection.prepare("SELECT sandbox_id,witness_json FROM controller_release_witnesses")?;
    let mut values = BTreeMap::new();
    for row in statement.query_map([], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, Vec<u8>>(1)?))
    })? {
        let (id, bytes) = row?;
        let witness: HostReleaseWitness = serde_json::from_slice(&bytes)?;
        let binding = witness.context.binding;
        if serde_json::to_vec(&witness)? != bytes
            || super::super::identity::sandbox_id(&namespace, binding.sandbox)?.as_str() != id
            || world.sandbox_record(binding.sandbox).is_none()
            || original_binding(world, binding.sandbox)? != Some(binding)
            || witness.input_sequence == 0
            || witness.input_sequence > allocators::input_high_water(connection)?
            || values.insert(binding.sandbox.0, witness).is_some()
        {
            return Err(JournalError::CorruptStore);
        }
    }
    Ok(values.into_values().collect())
}

pub(super) fn record_release(
    tx: &WorldTransaction<'_>,
    context: HostSlotContext,
    newly_confirmed: bool,
) -> Result<(), JournalError> {
    let world = tx.world();
    let existing = witnesses(tx, &world)?
        .into_iter()
        .find(|witness| witness.context.binding.sandbox == context.binding.sandbox);
    if !newly_confirmed {
        if existing.is_none_or(|witness| witness.context != context) {
            return Err(JournalError::CorruptStore);
        }
        return Ok(());
    }
    if existing.is_some() {
        return Err(JournalError::CorruptStore);
    }
    let sequence = allocators::input_high_water(tx)?;
    let bytes: Vec<u8> = tx.query_row(
        "SELECT input_json FROM controller_inputs WHERE sequence=?1",
        [sql(sequence)?],
        |row| row.get(0),
    )?;
    let entry: storage::StoredInput = serde_json::from_slice(&bytes)?;
    if !matches!(entry.input, core::ControllerInput::Maintenance(MaintenanceInput::Slot(core::maintenance::slots::SlotInput::ConfirmRelease {position,binding})) if position == context.position && binding == context.binding)
    {
        return Err(JournalError::CorruptStore);
    }
    store_witness(
        tx,
        HostReleaseWitness {
            context,
            input_sequence: sequence,
        },
    )
}

fn pinned(
    connection: &Connection,
    world: &core::World,
    namespace: &str,
    sandbox: core::SandboxId,
) -> Result<bool, JournalError> {
    if super::super::internal_inbox::pinned(connection, sandbox)?
        || super::super::lifecycle_inbox::pinned(connection, sandbox)?
    {
        return Ok(true);
    }
    let id = super::super::identity::sandbox_id(namespace, sandbox)?;
    let options: Option<Vec<u8>> = connection
        .query_row(
            "SELECT options_json FROM sandbox_create_options WHERE sandbox_id=?1",
            [id.as_str()],
            |row| row.get(0),
        )
        .optional()?;
    if let Some(bytes) = options {
        let options: super::super::CreateOptions = serde_json::from_slice(&bytes)?;
        options.validate().map_err(|_| JournalError::CorruptStore)?;
        if super::super::private_build::pinned(connection, world, namespace, sandbox, &options)? {
            return Ok(true);
        }
    }
    Ok(projections::publication_pins(connection)?.contains(&sandbox))
}

fn candidate(
    connection: &Connection,
    world: &core::World,
    sandbox: core::SandboxId,
) -> Result<Option<RetirementCandidate>, JournalError> {
    let Some(row) = world.sandbox_record(sandbox) else {
        return Ok(None);
    };
    if row.phase != core::Phase::Destroyed
        || pinned(connection, world, &namespace(connection)?, sandbox)?
    {
        return Ok(None);
    }
    let release = witnesses(connection, world)?
        .into_iter()
        .find(|witness| witness.context.binding.sandbox == sandbox);
    if release.is_none() {
        return Ok(None);
    }
    Ok(Some(RetirementCandidate {
        sandbox,
        owner: row.owner,
        epoch: row.epoch,
        operation: row.current_operation.ok_or(JournalError::CorruptStore)?.id,
        workspace: row.workspace,
        release,
    }))
}

fn request(candidate: &RetirementCandidate, retain_until: u64) -> RetireRequest {
    RetireRequest {
        sandbox: candidate.sandbox,
        owner: candidate.owner,
        epoch: candidate.epoch,
        operation: candidate.operation,
        retain_until,
        quiescence: QuiescenceCertificate {
            sandbox: candidate.sandbox,
            owner: candidate.owner,
            epoch: candidate.epoch,
            operation: candidate.operation,
            workspace: candidate.workspace,
            worker_drained: QuiescenceStatus::Confirmed,
            data_connections_drained: QuiescenceStatus::Confirmed,
            host_resources_reclaimed: QuiescenceStatus::Confirmed,
            external_pins_released: QuiescenceStatus::Confirmed,
        },
    }
}

fn delete_details(
    tx: &WorldTransaction<'_>,
    world: &core::World,
    namespace: &str,
    sandbox: core::SandboxId,
) -> Result<(), JournalError> {
    let id = super::super::identity::sandbox_id(namespace, sandbox)?;
    super::super::capacity::retire(tx, namespace, sandbox)?;
    tx.execute("DELETE FROM controller_receipt_history WHERE outbox_id IN (SELECT id FROM outbox WHERE sandbox_id=?1)", [id.as_str()])?;
    tx.execute("DELETE FROM outbox WHERE sandbox_id=?1", [id.as_str()])?;
    let mut logical = Vec::new();
    for effect in world
        .effect_ids()
        .iter()
        .filter(|effect| effect.command().sandbox() == sandbox)
    {
        if logical.contains(&effect.logical) {
            continue;
        }
        let bytes = serde_json::to_vec(&effect.logical)?;
        if tx.execute(
            "DELETE FROM controller_effect_keys WHERE logical_json=?1",
            [&bytes],
        )? != 1
        {
            return Err(JournalError::CorruptStore);
        }
        logical.push(effect.logical.clone());
    }
    for table in [
        "operations",
        "sandbox_lifetimes",
        "sandbox_create_options",
        "controller_data_credentials",
    ] {
        tx.execute(
            &format!("DELETE FROM {table} WHERE sandbox_id=?1"),
            [id.as_str()],
        )?;
    }
    if tx.execute(
        "DELETE FROM controller_release_witnesses WHERE sandbox_id=?1",
        [id.as_str()],
    )? != 1
        || tx.execute("DELETE FROM sandboxes WHERE sandbox_id=?1", [id.as_str()])? != 1
    {
        return Err(JournalError::CorruptStore);
    }
    allocators::remove_retired_scope(tx, sandbox)
}

impl ControllerJournal {
    /// Enumerate currently terminal, released, unpinned objects. Neither this
    /// read nor a candidate value grants authority to retire it later.
    /// # Errors
    /// Shared corruption aborts; missing physical release remains ineligible.
    pub fn retirement_candidates(&self) -> Result<Vec<RetirementCandidate>, JournalError> {
        self.committed_world
            .ensure_ready(&self.journal.connection)?;
        config(&self.journal.connection)?.ok_or(JournalError::UnsupportedStore)?;
        let tx = self.journal.connection.unchecked_transaction()?;
        let world = load_world(&tx)?;
        let mut values = Vec::new();
        for (sandbox, _) in world.sandbox_records() {
            if let Some(value) = candidate(&tx, &world, *sandbox)? {
                values.push(value);
            }
        }
        tx.commit()?;
        self.committed_world
            .ensure_ready(&self.journal.connection)?;
        Ok(values)
    }

    /// Retire only after the durable release protocol, current SQL pins and a
    /// trusted serialized worker/data/local-state verifier all agree. Guest
    /// or management request bodies must never supply this verifier.
    /// # Errors
    /// Any refusal rolls back retained results, input, counters and SQL closure.
    pub fn retire_reclaimed(
        &mut self,
        sandbox: core::SandboxId,
        now_ms: u64,
        verifier: impl FnOnce(&RetirementCandidate) -> Result<(), WorkerTransportError>,
        crashes: &mut impl CrashInjector,
    ) -> Result<(), JournalError> {
        let tx = WorldTransaction::begin(&mut self.journal.connection, &self.committed_world)?;
        let config = config(&tx)?.ok_or(JournalError::UnsupportedInput)?;
        let world = tx.world();
        projections::validate(&tx, &world, &self.namespace)?;
        let candidate = candidate(&tx, &world, sandbox)?.ok_or(JournalError::UnresolvedAction)?;
        let now_ms = now_ms.max(
            world
                .maintenance_state()
                .ok_or(JournalError::CorruptStore)?
                .time_floor(),
        );
        let retain_until = now_ms
            .checked_add(config.retention_ms)
            .ok_or(JournalError::MaintenanceCapacity)?;
        verifier(&candidate).map_err(|_| JournalError::UnresolvedAction)?;
        storage::apply_maintenance(&tx, MaintenanceInput::ExpireResults { now_ms })?;
        storage::apply_maintenance(
            &tx,
            MaintenanceInput::Retire(request(&candidate, retain_until)),
        )?;
        delete_details(&tx, &world, &self.namespace, sandbox)?;
        crashes.hit(CrashPoint::AfterRetirementDelete)?;
        crashes.hit(CrashPoint::BeforeCommit)?;
        tx.commit()?;
        crashes.hit(CrashPoint::AfterCommit)
    }

    /// Persist the monotone time floor and release only expired result records.
    /// # Errors
    /// Capacity/commit errors do not report expiry as durably completed.
    pub fn expire_retained(
        &mut self,
        now_ms: u64,
        crashes: &mut impl CrashInjector,
    ) -> Result<usize, JournalError> {
        let tx = WorldTransaction::begin(&mut self.journal.connection, &self.committed_world)?;
        config(&tx)?.ok_or(JournalError::UnsupportedInput)?;
        let world = tx.world();
        let state = world
            .maintenance_state()
            .ok_or(JournalError::CorruptStore)?;
        let now_ms = now_ms.max(state.time_floor());
        let before = state.retained_count();
        if before == 0
            || !state
                .retained_results()
                .iter()
                .any(|(_, result)| result.expires_at <= now_ms)
        {
            return Ok(0);
        }
        let decision = storage::apply_maintenance(&tx, MaintenanceInput::ExpireResults { now_ms })?;
        let after = decision
            .next
            .maintenance_state()
            .ok_or(JournalError::CorruptStore)?
            .retained_count();
        crashes.hit(CrashPoint::BeforeCommit)?;
        tx.commit()?;
        crashes.hit(CrashPoint::AfterCommit)?;
        Ok(before - after)
    }
}
