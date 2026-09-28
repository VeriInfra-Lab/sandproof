use super::identity;
use crate::{AcceptedWrite, JournalError, journal::write_accepted};
use rusqlite::{Connection, OptionalExtension, Transaction, params};
use sandbox_controller_core as core;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct StoredInput {
    version: u16,
    input: core::ControllerInput,
}

pub(super) fn load_world(connection: &Connection) -> Result<core::World, JournalError> {
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

pub(super) fn apply(
    transaction: &Transaction<'_>,
    namespace: &str,
    world: &core::World,
    input: core::ControllerInput,
) -> Result<core::ControllerDecision, JournalError> {
    let sandbox = identity::input_sandbox(&input)?;
    let serialized = serde_json::to_vec(&StoredInput {
        version: 1,
        input: input.clone(),
    })?;
    let decision =
        core::handle_controller(world, input).map_err(JournalError::ControllerRejected)?;
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
    let actions = identity::new_actions(transaction, namespace, world, &decision.next, sandbox)?;
    write_accepted(
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
    )?;
    identity::update_operation_projections(transaction, namespace, &decision.next, sandbox)?;
    transaction.execute(
        "INSERT INTO controller_inputs(input_json) VALUES (?1)",
        params![serialized],
    )?;
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
