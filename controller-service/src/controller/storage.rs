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
    if checkpoint::config(connection)?.is_some() {
        return checkpoint::load_world(connection);
    }
    let mut statement = connection
        .prepare("SELECT sequence, input_json FROM controller_inputs ORDER BY sequence")?;
    let rows = statement.query_map([], |row| {
        Ok((row.get::<_, i64>(0)?, row.get::<_, Vec<u8>>(1)?))
    })?;
    let mut world = core::initial_world();
    let mut expected = 1_i64;
    for row in rows {
        let (sequence, json) = row?;
        let entry: StoredInput = serde_json::from_slice(&json)?;
        if sequence != expected || entry.version != 1 {
            return Err(JournalError::CorruptStore);
        }
        expected = expected.checked_add(1).ok_or(JournalError::CorruptStore)?;
        world = core::handle_controller(&world, entry.input)
            .map_err(|_| JournalError::CorruptStore)?
            .next;
    }
    Ok(world)
}

// Opening validation binds SQL-only predecessor context to the exact accepted
// Bind inputs, rather than trusting a merely plausible smaller VM number.
pub(super) fn replayed_host_contexts(
    connection: &Connection,
) -> Result<Vec<super::HostSlotContext>, JournalError> {
    if checkpoint::config(connection)?.is_some() {
        return checkpoint::replayed_contexts(connection);
    }
    let mut statement =
        connection.prepare("SELECT input_json FROM controller_inputs ORDER BY sequence")?;
    let mut contexts = std::collections::BTreeMap::new();
    for bytes in statement.query_map([], |row| row.get::<_, Vec<u8>>(0))? {
        let entry: StoredInput = serde_json::from_slice(&bytes?)?;
        if let core::ControllerInput::Maintenance(core::maintenance::MaintenanceInput::Slot(
            core::maintenance::slots::SlotInput::Bind {
                position,
                expected_previous,
                binding,
            },
        )) = entry.input
        {
            contexts.insert(
                position.0,
                super::HostSlotContext {
                    position,
                    expected_previous,
                    binding,
                },
            );
        }
    }
    Ok(contexts.into_values().collect())
}

pub(super) fn apply(
    transaction: &WorldTransaction<'_>,
    namespace: &str,
    input: core::ControllerInput,
) -> Result<core::ControllerDecision, JournalError> {
    let world = transaction.world();
    let sandbox = identity::input_sandbox(&input)?;
    let serialized = checkpoint::input_bytes(
        transaction,
        &StoredInput {
            version: 1,
            input: input.clone(),
        },
    )?;
    checkpoint::require_input_room(transaction, serialized.len())?;
    let decision =
        core::handle_controller(&world, input).map_err(JournalError::ControllerRejected)?;
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
    let decision = append_maintenance(transaction, &transaction.world(), input)?;
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
    })
}

#[cfg(test)]
thread_local! {
    pub(super) static FULL_REPLAYS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}
