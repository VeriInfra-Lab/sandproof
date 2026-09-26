//! V1 orchestration calls the established reducer for every state change.
//! Physical success is still owned by a command-specific trusted host worker.

use crate::{
    ControllerJournal, DispatchResult, HostEvidence, JournalError, NeverCrash, WorkerTransportError,
};
use sandbox_controller_core as core;
use sandproof_protocol::{WorkAction, WorkReceipt};
use serde::{Serialize, de::DeserializeOwned};

fn drain<E, W>(
    journal: &mut ControllerJournal,
    sandbox: core::SandboxId,
    worker: &mut W,
) -> Result<(), JournalError>
where
    E: HostEvidence + Serialize + DeserializeOwned + PartialEq,
    W: FnMut(&WorkAction<core::EffectId>) -> Result<WorkReceipt<E>, WorkerTransportError>,
{
    loop {
        match journal.dispatch_next_for(sandbox, worker, &mut NeverCrash)? {
            DispatchResult::NoReadyAction => return Ok(()),
            DispatchResult::ReceiptRecorded(_) => {}
            DispatchResult::OutcomeUnknown { .. } => return Err(JournalError::UnresolvedAction),
        }
    }
}

/// Accept a V1 create, install its fence, reserve a VM, and publish only after
/// all physical command receipts satisfy the core's Running preconditions.
/// The caller must authenticate the actor and allocate unique IDs first.
///
/// # Errors
/// Any rejected transition, uncertain host action, or failed publication is
/// left durably visible. This function never treats a VM launch as readiness.
pub fn create_v1<E, W>(
    journal: &mut ControllerJournal,
    request: core::CreateRequest,
    worker: &mut W,
) -> Result<core::World, JournalError>
where
    E: HostEvidence + Serialize + DeserializeOwned + PartialEq,
    W: FnMut(&WorkAction<core::EffectId>) -> Result<WorkReceipt<E>, WorkerTransportError>,
{
    let sandbox = request.sandbox;
    let key = core::OperationKey {
        sandbox,
        operation: request.operation,
    };
    journal.apply(
        core::ControllerInput::Request(core::Request::Create(request)),
        &mut NeverCrash,
    )?;
    let reserved = journal.apply(
        core::ControllerInput::Scheduler(core::SchedulerInput::ReserveStartup(key)),
        &mut NeverCrash,
    )?;
    drain(journal, sandbox, worker)?;
    let plan = reserved
        .next
        .operation_plan(key)
        .ok_or(JournalError::CorruptStore)?;
    journal.apply(
        core::ControllerInput::Scheduler(core::SchedulerInput::DispatchPrepare(
            sandbox,
            plan.vm.ok_or(JournalError::CorruptStore)?,
            plan.slot.ok_or(JournalError::CorruptStore)?,
        )),
        &mut NeverCrash,
    )?;
    drain(journal, sandbox, worker)?;
    Ok(journal
        .apply(
            core::ControllerInput::Publication(core::PublicationInput::Running(sandbox)),
            &mut NeverCrash,
        )?
        .next)
}

/// Revoke access, stop every planned live VM and publish destruction only when
/// the host's exact stop receipt satisfies the core. Cleanup follows release.
///
/// # Errors
/// An uncertain or failed stop leaves the sandbox unreported as Destroyed.
pub fn kill_v1<E, W>(
    journal: &mut ControllerJournal,
    request: core::KillRequest,
    worker: &mut W,
) -> Result<core::World, JournalError>
where
    E: HostEvidence + Serialize + DeserializeOwned + PartialEq,
    W: FnMut(&WorkAction<core::EffectId>) -> Result<WorkReceipt<E>, WorkerTransportError>,
{
    let sandbox = request.sandbox;
    let operation = request.operation;
    let accepted = journal.apply(
        core::ControllerInput::Request(core::Request::Kill(request)),
        &mut NeverCrash,
    )?;
    drain(journal, sandbox, worker)?;
    let record = accepted
        .next
        .sandbox_record(sandbox)
        .ok_or(JournalError::CorruptStore)?;
    let key = core::OperationKey { sandbox, operation };
    let plan = accepted
        .next
        .operation_plan(key)
        .ok_or(JournalError::CorruptStore)?;
    for vm in &plan.stop_vms {
        journal.apply(
            core::ControllerInput::Scheduler(core::SchedulerInput::StopVm(
                sandbox,
                record.generation,
                *vm,
            )),
            &mut NeverCrash,
        )?;
        drain(journal, sandbox, worker)?;
    }
    let published = journal.apply(
        core::ControllerInput::Publication(core::PublicationInput::Destroyed(sandbox)),
        &mut NeverCrash,
    )?;
    let resources = published.next.resource_index(sandbox);
    for vm in resources.vms {
        journal.apply(
            core::ControllerInput::Scheduler(core::SchedulerInput::CleanupVm(
                sandbox,
                record.generation,
                vm,
            )),
            &mut NeverCrash,
        )?;
        drain(journal, sandbox, worker)?;
    }
    for slot in resources.slots {
        journal.apply(
            core::ControllerInput::Scheduler(core::SchedulerInput::CleanupSlot(
                sandbox,
                record.generation,
                slot,
            )),
            &mut NeverCrash,
        )?;
        drain(journal, sandbox, worker)?;
    }
    journal.world()
}
