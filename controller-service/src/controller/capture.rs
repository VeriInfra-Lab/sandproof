//! Original receipt source used by capture admission, not a new VM authority.
use super::{ControllerJournal, identity};
use crate::{HostEvidence, JournalError, capture::SourceProcessIdentity};
use rusqlite::Transaction;
use sandbox_controller_core as core;
use sandproof_protocol::{WorkAction, WorkOutcome, WorkReceipt};
use serde::de::DeserializeOwned;

fn agree(
    slot: &mut Option<SourceProcessIdentity>,
    observed: SourceProcessIdentity,
) -> Result<(), JournalError> {
    if observed.pid == 0 || observed.boot_id.is_empty() {
        return Err(JournalError::UnresolvedAction);
    }
    match slot {
        Some(previous) if previous != &observed => Err(JournalError::UnresolvedAction),
        Some(_) => Ok(()),
        None => {
            *slot = Some(observed);
            Ok(())
        }
    }
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
    let mut prepared = None;
    let mut activated = None;
    for row in rows {
        let (action_bytes, receipt_bytes) = row?;
        let action: WorkAction<core::EffectId> = serde_json::from_slice(&action_bytes)?;
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
            _ => continue,
        };
        if generation != current.generation || found_vm != vm {
            continue;
        }
        if !core::effect::effect_succeeded_check(world, command) {
            continue;
        }
        let logical = world
            .logical_effect(&action.command.logical)
            .ok_or(JournalError::CorruptStore)?;
        if logical.current_attempt != action.command.attempt {
            continue;
        }
        let core::EffectOrigin::Operation(key) = logical.origin else {
            return Err(JournalError::CorruptStore);
        };
        let plan = world
            .operation_plan(key)
            .ok_or(JournalError::CorruptStore)?;
        if key.sandbox != sandbox
            || (plan.vm != Some(vm)
                && !core::capture::continuation::activation_belongs_to_continuation(plan, command))
            || plan.generation != generation
            || command.epoch() != Some(plan.epoch)
            || action.identity.owner != identity::owner_id(current.owner)?
            || action.identity.sandbox != public_sandbox
            || action.identity.operation != identity::operation_id(namespace, key)?
            || action.identity.epoch != plan.epoch
            || action.identity.attempt != action.command.attempt
            || action.identity.generation != generation
            || action.identity.resource != identity::resource(namespace, command)?
        {
            return Err(JournalError::CorruptStore);
        }
        let receipt: WorkReceipt<E> = serde_json::from_slice(&receipt_bytes)?;
        receipt.validate_for(&action)?;
        let WorkOutcome::ObservedSuccess(evidence) = receipt.outcome else {
            return Err(JournalError::UnresolvedAction);
        };
        if !evidence.validates(command) {
            return Err(JournalError::InvalidHostEvidence);
        }
        let source = evidence
            .source_process_identity(command, &action.identity.resource)
            .ok_or(JournalError::UnresolvedAction)?;
        let observed = if is_prepare { &mut prepared } else { &mut activated };
        agree(observed, source)?;
    }
    match (prepared, activated) {
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
