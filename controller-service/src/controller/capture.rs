//! Original receipt source used by capture admission, not a new VM authority.
use super::{ControllerJournal, identity};
use crate::{HostEvidence, JournalError, capture::SourceProcessIdentity};
use rusqlite::Transaction;
use sandbox_controller_core as core;
use sandproof_protocol::{WorkAction, WorkOutcome, WorkReceipt};
use serde::de::DeserializeOwned;

/// Accumulated observations retain their original order and full identity.
struct OriginalSourceRows {
    prepared: Option<SourceProcessIdentity>,
    activated: Option<SourceProcessIdentity>,
}

trait OriginalRowCodec<E> {
    fn decode_action(
        &mut self,
        bytes: &[u8],
    ) -> Result<WorkAction<core::EffectId>, serde_json::Error>;

    fn decode_receipt(&mut self, bytes: &[u8]) -> Result<WorkReceipt<E>, serde_json::Error>;
}

struct JsonOriginalRowCodec;

impl<E: DeserializeOwned> OriginalRowCodec<E> for JsonOriginalRowCodec {
    fn decode_action(
        &mut self,
        bytes: &[u8],
    ) -> Result<WorkAction<core::EffectId>, serde_json::Error> {
        serde_json::from_slice(bytes)
    }

    fn decode_receipt(&mut self, bytes: &[u8]) -> Result<WorkReceipt<E>, serde_json::Error> {
        serde_json::from_slice(bytes)
    }
}

fn agree(
    slot: Option<SourceProcessIdentity>,
    observed: SourceProcessIdentity,
) -> (Result<(), JournalError>, Option<SourceProcessIdentity>) {
    if observed.pid == 0 || observed.boot_id.is_empty() {
        return (Err(JournalError::UnresolvedAction), slot);
    }
    match slot {
        Some(previous) => {
            let result = if previous == observed {
                Ok(())
            } else {
                Err(JournalError::UnresolvedAction)
            };
            (result, Some(previous))
        }
        None => (Ok(()), Some(observed)),
    }
}

fn accept_original_source_row<E: HostEvidence, D: OriginalRowCodec<E>>(
    row: rusqlite::Result<(Vec<u8>, Vec<u8>)>,
    world: &core::World,
    namespace: &str,
    current: &core::SandboxRecord,
    binding: (core::SandboxId, core::VmId, &sandproof_protocol::SandboxId),
    mut rows: OriginalSourceRows,
    codec: &mut D,
) -> (Result<(), JournalError>, OriginalSourceRows) {
    let (sandbox, vm, public_sandbox) = binding;
    let (action_bytes, receipt_bytes) = match row {
        Ok(value) => value,
        Err(error) => return (Err(error.into()), rows),
    };
    let action = match codec.decode_action(&action_bytes) {
        Ok(value) => value,
        Err(error) => return (Err(error.into()), rows),
    };
    let command = action.command.command();
    let (is_prepare, generation, found_vm) = match command {
        core::Command::PrepareVm {
            sandbox: found,
            generation,
            vm,
            ..
        } if *found == sandbox => (true, *generation, *vm),
        core::Command::ActivateVm {
            sandbox: found,
            generation,
            vm,
            ..
        } if *found == sandbox => (false, *generation, *vm),
        _ => return (Ok(()), rows),
    };
    if generation != current.generation || found_vm != vm {
        return (Ok(()), rows);
    }
    if !core::effect::effect_succeeded_check(world, command) {
        return (Ok(()), rows);
    }
    let Some(logical) = world.logical_effect(&action.command.logical) else {
        return (Err(JournalError::CorruptStore), rows);
    };
    if logical.current_attempt != action.command.attempt {
        return (Ok(()), rows);
    }
    let core::EffectOrigin::Operation(key) = logical.origin else {
        return (Err(JournalError::CorruptStore), rows);
    };
    let Some(plan) = world.operation_plan(key) else {
        return (Err(JournalError::CorruptStore), rows);
    };
    if key.sandbox != sandbox
        || (plan.vm != Some(vm)
            && !core::capture::continuation::activation_belongs_to_continuation(plan, command))
        || plan.generation != generation
        || command.epoch() != Some(plan.epoch)
        || action.identity.owner
            != match identity::owner_id(current.owner) {
                Ok(value) => value,
                Err(error) => return (Err(error), rows),
            }
        || &action.identity.sandbox != public_sandbox
        || action.identity.operation
            != match identity::operation_id(namespace, key) {
                Ok(value) => value,
                Err(error) => return (Err(error), rows),
            }
        || action.identity.epoch != plan.epoch
        || action.identity.attempt != action.command.attempt
        || action.identity.generation != generation
        || action.identity.resource
            != match identity::resource(namespace, command) {
                Ok(value) => value,
                Err(error) => return (Err(error), rows),
            }
    {
        return (Err(JournalError::CorruptStore), rows);
    }
    let receipt = match codec.decode_receipt(&receipt_bytes) {
        Ok(value) => value,
        Err(error) => return (Err(error.into()), rows),
    };
    if let Err(error) = receipt.validate_for(&action) {
        return (Err(error.into()), rows);
    }
    let WorkOutcome::ObservedSuccess(evidence) = receipt.outcome else {
        return (Err(JournalError::UnresolvedAction), rows);
    };
    if !evidence.validates(command) {
        return (Err(JournalError::InvalidHostEvidence), rows);
    }
    let Some(source) = evidence.source_process_identity(command, &action.identity.resource) else {
        return (Err(JournalError::UnresolvedAction), rows);
    };
    let result;
    if is_prepare {
        (result, rows.prepared) = agree(rows.prepared, source);
    } else {
        (result, rows.activated) = agree(rows.activated, source);
    }
    (result, rows)
}

// Retain lazy iteration and every callback outcome, including row errors.
fn accept_original_rows<I, F>(
    rows: I,
    mut state: OriginalSourceRows,
    mut accept: F,
) -> (Result<(), JournalError>, OriginalSourceRows)
where
    I: Iterator,
    F: FnMut(I::Item, OriginalSourceRows) -> (Result<(), JournalError>, OriginalSourceRows),
{
    let mut row_error = None;
    for row in rows {
        let (result, next_state) = accept(row, state);
        state = next_state;
        if let Err(error) = result {
            row_error = Some(error);
            break;
        }
    }
    let result = match row_error {
        Some(error) => Err(error),
        None => Ok(()),
    };
    (result, state)
}

/// Read only already committed original observations under the same SQL
/// snapshot as the candidate. Admission will persist the resulting identity
/// with its original capture work before any physical capture call.
pub(super) fn original_source<E: HostEvidence + DeserializeOwned>(
    transaction: &Transaction<'_>,
    namespace: &str,
    world: &core::World,
    sandbox: core::SandboxId,
) -> Result<SourceProcessIdentity, JournalError> {
    original_source_using::<E, _>(transaction, namespace, world, sandbox, JsonOriginalRowCodec)
}

fn original_source_using<E: HostEvidence, D: OriginalRowCodec<E>>(
    transaction: &Transaction<'_>,
    namespace: &str,
    world: &core::World,
    sandbox: core::SandboxId,
    mut codec: D,
) -> Result<SourceProcessIdentity, JournalError> {
    let current = world
        .sandbox_record(sandbox)
        .ok_or(JournalError::UnsupportedInput)?;
    if current.phase != core::Phase::Running || current.terminal {
        return Err(JournalError::UnsupportedInput);
    }
    let vm = current.current_vm.ok_or(JournalError::CorruptStore)?;
    let physical = world.vm_record(vm).ok_or(JournalError::CorruptStore)?;
    if physical.owner != current.owner
        || physical.sandbox != sandbox
        || physical.generation != current.generation
        || physical.status != core::VmStatus::Executing
    {
        return Err(JournalError::CorruptStore);
    }
    let mut statement = transaction.prepare(
        "SELECT action_json,receipt_json FROM outbox
         WHERE sandbox_id=?1 AND dispatch_state='receipted' ORDER BY id",
    )?;
    let public_sandbox = identity::sandbox_id(namespace, sandbox)?;
    let rows = statement.query_map([public_sandbox.as_str()], |row| {
        Ok((row.get::<_, Vec<u8>>(0)?, row.get::<_, Vec<u8>>(1)?))
    })?;
    let source_rows = OriginalSourceRows {
        prepared: None,
        activated: None,
    };
    let binding = (sandbox, vm, &public_sandbox);
    let (result, source_rows) = accept_original_rows(rows, source_rows, move |row, state| {
        accept_original_source_row::<E, _>(
            row, world, namespace, current, binding, state, &mut codec,
        )
    });
    result?;
    match (source_rows.prepared, source_rows.activated) {
        (Some(prepared), Some(activated)) if prepared == activated => Ok(prepared),
        _ => Err(JournalError::UnresolvedAction),
    }
}

impl ControllerJournal {
    /// Read the original VMM identity from consistent committed Prepare and
    /// Activate receipts. This is neither admission nor proof of current life.
    /// Capture must still recheck the original process before its first effect.
    ///
    /// # Errors
    /// Missing/old/contradictory source remains unresolved; wrong-phase or
    /// corrupt original bindings are refused without changing state.
    pub fn source_for_capture<E: HostEvidence + DeserializeOwned>(
        &self,
        sandbox: core::SandboxId,
    ) -> Result<SourceProcessIdentity, JournalError> {
        let world = self.world()?;
        let transaction = self.journal.connection.unchecked_transaction()?;
        self.committed_world.check_identity(&transaction)?;
        let source = original_source::<E>(&transaction, &self.namespace, &world, sandbox)?;
        self.committed_world.check_identity(&transaction)?;
        transaction.commit()?;
        Ok(source)
    }
}

#[cfg(test)]
mod tests;
