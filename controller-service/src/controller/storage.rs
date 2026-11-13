use super::{
    allocators::{self, SqlId},
    checkpoint, identity,
    transaction::WorldTransaction,
};
use crate::{AcceptedWrite, JournalError, journal::write_accepted_with_ids};
use rusqlite::{Connection, OptionalExtension, params};
use sandbox_controller_core as core;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct StoredInput {
    pub(super) version: u16,
    pub(super) input: core::ControllerInput,
}

pub(super) fn load_world(connection: &Connection) -> Result<core::World, JournalError> {
    #[cfg(test)]
    FULL_REPLAYS.with(|count| count.set(count.get() + 1));
    if connection.is_autocommit() {
        let tx = connection.unchecked_transaction()?;
        let world = load_world_snapshot(&tx)?;
        tx.commit()?;
        return Ok(world);
    }
    load_world_snapshot(connection)
}

fn load_world_snapshot(connection: &Connection) -> Result<core::World, JournalError> {
    checkpoint::load_world(connection)
}

pub(super) fn apply(
    transaction: &WorldTransaction<'_>,
    namespace: &str,
    input: core::ControllerInput,
) -> Result<core::ControllerDecision, JournalError> {
    let world = transaction.world();
    checkpoint::check_input_profile(transaction, &world, &input)?;
    let sandbox = identity::input_sandbox(&input)?;
    let stored = StoredInput { version: 1, input };
    let serialized = checkpoint::input_bytes(transaction, &stored)?;
    let decision = core::handle_controller(&world, stored.input.clone())
        .map_err(JournalError::ControllerRejected)?;
    let full_capacity = super::capacity::config(transaction)?.is_some();
    if full_capacity && decision.next == world {
        return Ok(decision);
    }
    if let core::ControllerInput::Request(core::Request::Create(request)) = &stored.input {
        super::capacity::admit(transaction, request)?;
    }
    let before = super::capacity::cursors(transaction)?;
    checkpoint::require_input_room(transaction, serialized.len())?;
    let record = decision
        .next
        .sandbox_record(sandbox)
        .ok_or(JournalError::CorruptStore)?;
    let operation = record.current_operation.ok_or(JournalError::CorruptStore)?;
    let key = core::OperationKey {
        sandbox,
        operation: operation.id,
    };
    let ledger = decision
        .next
        .operation_record(key)
        .ok_or(JournalError::CorruptStore)?;
    if world.operation_record(key).is_none() {
        super::capacity::require_operation_room(transaction, &world, key, ledger.operation.kind)?;
        allocators::require_available(transaction)?;
    }
    allocators::record_accepted(transaction, &world, sandbox, operation.id)?;
    let sequence = allocators::next_sql(transaction, SqlId::Input)?;
    let public_sandbox = identity::sandbox_id(namespace, sandbox)?;
    let revision: Option<i64> = transaction
        .query_row(
            "SELECT revision FROM sandboxes WHERE sandbox_id=?1",
            [public_sandbox.as_str()],
            |row| row.get(0),
        )
        .optional()?;
    let revision = revision
        .map(u64::try_from)
        .transpose()
        .map_err(|_| JournalError::CorruptStore)?;
    let actions = identity::new_actions(transaction, namespace, &world, &decision.next, sandbox)?;
    let outbox_ids = allocators::reserve_sql(transaction, SqlId::Outbox, actions.len())?;
    write_accepted_with_ids(
        transaction,
        &AcceptedWrite {
            owner: identity::owner_id(record.owner)?,
            sandbox: public_sandbox,
            operation: identity::operation_id(namespace, key)?,
            expected_revision: revision,
            sandbox_state: serde_json::to_vec(record)?,
            operation_state: serde_json::to_vec(ledger)?,
            actions,
        },
        Some(&outbox_ids),
    )?;
    identity::update_operation_projections(transaction, namespace, &decision.next, sandbox)?;
    transaction.execute(
        "INSERT INTO controller_inputs(sequence,input_json) VALUES (?1,?2)",
        params![sequence, serialized],
    )?;
    super::capacity::spend(transaction, sandbox, before)?;
    transaction.advance(decision.next.clone());
    Ok(decision)
}

// This deliberately bypasses sandbox/outbox projection writers: maintenance
// changes no lifecycle operation and can later remove its sandbox entirely.
pub(super) fn append_maintenance(
    transaction: &rusqlite::Transaction<'_>,
    world: &core::World,
    input: core::maintenance::MaintenanceInput,
) -> Result<core::ControllerDecision, JournalError> {
    let input = core::ControllerInput::Maintenance(input);
    let decision =
        core::handle_controller(world, input.clone()).map_err(JournalError::ControllerRejected)?;
    if super::capacity::config(transaction)?.is_some() && decision.next == *world {
        return Ok(decision);
    }
    let serialized = checkpoint::input_bytes(transaction, &StoredInput { version: 1, input })?;
    checkpoint::require_input_room(transaction, serialized.len())?;
    let sequence = allocators::next_sql(transaction, SqlId::Input)?;
    transaction.execute(
        "INSERT INTO controller_inputs(sequence,input_json) VALUES (?1,?2)",
        params![sequence, serialized],
    )?;
    Ok(decision)
}

pub(super) fn apply_maintenance(
    transaction: &WorldTransaction<'_>,
    input: core::maintenance::MaintenanceInput,
) -> Result<core::ControllerDecision, JournalError> {
    let sandbox = match &input {
        core::maintenance::MaintenanceInput::Retire(request) => Some(request.sandbox),
        core::maintenance::MaintenanceInput::Slot(
            core::maintenance::slots::SlotInput::Bind { binding, .. }
            | core::maintenance::slots::SlotInput::BeginRelease { binding, .. }
            | core::maintenance::slots::SlotInput::ConfirmRelease { binding, .. }
            | core::maintenance::slots::SlotInput::CheckOccupied { binding, .. },
        ) => Some(binding.sandbox),
        _ => None,
    };
    let before = super::capacity::cursors(transaction)?;
    let decision = append_maintenance(transaction, &transaction.world(), input)?;
    if let Some(sandbox) = sandbox {
        super::capacity::spend(transaction, sandbox, before)?;
    }
    transaction.advance(decision.next.clone());
    Ok(decision)
}

pub(super) fn unresolved(world: &core::World, sandbox: core::SandboxId) -> bool {
    world.effect_ids().iter().any(|effect| {
        effect.command().sandbox() == sandbox
            && matches!(
                world.effect_status(effect),
                Some(core::EffectStatus::Dispatched | core::EffectStatus::OutcomeUnknown)
            )
            && !core::maintenance::released_execution::discharged(world, effect)
    })
}

#[cfg(test)]
thread_local! {
    pub(super) static FULL_REPLAYS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}
