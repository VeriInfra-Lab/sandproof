//! Only the original writer can discharge an already released execution's
//! terminal fence. No Node receipt, remote flag or current Free cell supplies
//! the historical ConfirmRelease witness required here.
use super::{ControllerJournal, codec};
use crate::{CrashInjector, CrashPoint, JournalError};
use crate::controller::{checkpoint, storage, transaction::WorldTransaction};
use rusqlite::Connection;
use sandbox_controller_core as core;
use core::maintenance::released_execution::ReleasedExecution;
use sandproof_protocol::lifecycle::LifecycleOperation;

pub(in crate::controller) fn permits(
    connection: &Connection,
    world: &core::World,
    basis: ReleasedExecution,
) -> Result<bool, JournalError> {
    if !core::maintenance::released_execution::valid_basis(world, basis) { return Ok(false); }
    let Some(inbox) = codec::load(connection)? else { return Ok(false) };
    let Some(session) = inbox.sessions.iter().find(|s|
        s.local.is_some_and(|local| local.sandbox == basis.key.sandbox)) else { return Ok(false) };
    let Some(local) = session.local else { return Ok(false) };
    let Some(entry) = session.entries.last() else { return Ok(false) };
    if !session.terminal || local.owner != basis.binding.owner
        || entry.local_operation != Some(basis.key.operation)
        || entry.intent.accepted_version != session.current_version
        || entry.intent.request.binding.session != session.id
        || entry.intent.request.binding.key.owner != session.owner
        || entry.intent.request.binding.node != inbox.config.node
        || entry.intent.center != inbox.config.center || entry.intent.retention != session.retention
        || !matches!(entry.intent.request.operation, LifecycleOperation::Destroy {})
    { return Ok(false); }
    let Some(witness) = checkpoint::release_witnesses(connection, world)?.into_iter()
        .find(|w| w.context.binding.sandbox == basis.key.sandbox) else { return Ok(false) };
    if witness.context.binding != basis.binding || witness.context.position != basis.position
        || witness.context.expected_previous != basis.expected_previous
        || witness.input_sequence != basis.release_input_sequence
    { return Ok(false); }
    super::source_work::released_execution_source(connection, world, basis.binding)
}

pub(in crate::controller) fn validate_saved(
    connection: &Connection,
    world: &core::World,
) -> Result<(), JournalError> {
    for id in world.effect_ids() {
        if let Some(core::LogicalEffectResult::ReleasedExecution(basis)) =
            world.logical_effect(&id.logical).and_then(|r| r.result)
            && !permits(connection, world, basis)?
        { return Err(JournalError::CorruptStore); }
    }
    Ok(())
}

impl ControllerJournal {
    /// Try only the current formal Destroy of a fully released Unload. The
    /// original unknown receipt/status remains unchanged. Material retirement,
    /// public Cleaned and quota settlement remain separate obligations.
    pub(crate) fn close_released_lifecycle_execution(
        &mut self,
        sandbox: core::SandboxId,
        crashes: &mut impl CrashInjector,
    ) -> Result<bool, JournalError> {
        let tx = WorldTransaction::begin(&mut self.journal.connection, &self.committed_world)?;
        let world = tx.world();
        let Some(row) = world.sandbox_record(sandbox) else { return Ok(false) };
        let Some(operation) = row.current_operation else { return Ok(false) };
        if row.phase != core::Phase::Revoking || operation.kind != core::OperationKind::Kill
            || core::maintenance::released_execution::terminal_closed(&world, sandbox)
        { return Ok(false); }
        let Some(witness) = checkpoint::release_witnesses(&tx, &world)?.into_iter()
            .find(|w| w.context.binding.sandbox == sandbox) else { return Ok(false) };
        let basis = ReleasedExecution {
            key: core::OperationKey { sandbox, operation: operation.id }, epoch: row.epoch,
            position: witness.context.position, expected_previous: witness.context.expected_previous,
            binding: witness.context.binding, release_input_sequence: witness.input_sequence,
        };
        if !permits(&tx, &world, basis)? { return Ok(false); }
        storage::apply(&tx, &self.namespace, core::ControllerInput::Maintenance(
            core::maintenance::MaintenanceInput::CloseReleasedExecution(basis)))?;
        crashes.hit(CrashPoint::BeforeCommit)?;
        tx.commit()?;
        crashes.hit(CrashPoint::AfterCommit)?;
        Ok(true)
    }
}
