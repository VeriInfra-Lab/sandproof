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
        if journal
            .uncertain_actions()?
            .iter()
            .any(|action| action.command.command().sandbox() == sandbox)
        {
            return Err(JournalError::UnresolvedAction);
        }
        match journal.dispatch_next_for(sandbox, worker, &mut NeverCrash)? {
            DispatchResult::NoReadyAction => return Ok(()),
            DispatchResult::ReceiptRecorded(_) => {}
            DispatchResult::OutcomeUnknown { .. } => return Err(JournalError::UnresolvedAction),
        }
    }
}

fn scheduled(world: &core::World, command: &core::Command) -> bool {
    world
        .effect_ids()
        .iter()
        .any(|effect| effect.command() == command)
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
    let operation = request.operation;
    journal.apply(
        core::ControllerInput::Request(core::Request::Create(request)),
        &mut NeverCrash,
    )?;
    finish_admitted_create_v1(journal, sandbox, operation, worker)
}

/// Continue only an already durably admitted V1 create. This is the bridge
/// from atomic lease/quota admission to the unchanged reducer workflow.
/// A fresh call must not be used as automatic recovery after an uncertain
/// worker result; that requires explicit host reconciliation.
///
/// # Errors
/// Rejects missing/wrong-phase intent or any unresolved physical action.
pub fn finish_admitted_create_v1<E, W>(
    journal: &mut ControllerJournal,
    sandbox: core::SandboxId,
    operation: core::OperationId,
    worker: &mut W,
) -> Result<core::World, JournalError>
where
    E: HostEvidence + Serialize + DeserializeOwned + PartialEq,
    W: FnMut(&WorkAction<core::EffectId>) -> Result<WorkReceipt<E>, WorkerTransportError>,
{
    let world = journal.world()?;
    let record = world
        .sandbox_record(sandbox)
        .ok_or(JournalError::UnsupportedInput)?;
    if record.phase != core::Phase::Starting
        || !matches!(record.current_operation, Some(current)
            if current.id == operation
                && current.kind == core::OperationKind::Create
                && matches!(current.status, core::OperationStatus::Pending | core::OperationStatus::OutcomeUnknown))
    {
        return Err(JournalError::UnsupportedInput);
    }
    let key = core::OperationKey { sandbox, operation };
    if world
        .operation_plan(key)
        .is_some_and(|plan| plan.vm.is_none())
    {
        journal.apply(
            core::ControllerInput::Scheduler(core::SchedulerInput::ReserveStartup(key)),
            &mut NeverCrash,
        )?;
    }
    drain(journal, sandbox, worker)?;
    let refreshed = journal.world()?;
    let plan = refreshed
        .operation_plan(key)
        .ok_or(JournalError::CorruptStore)?;
    let vm = plan.vm.ok_or(JournalError::CorruptStore)?;
    let slot = plan.slot.ok_or(JournalError::CorruptStore)?;
    let prepare = core::Command::PrepareVm {
        sandbox,
        epoch: plan.epoch,
        generation: plan.generation,
        vm,
        slot,
        source: plan.restore_from,
    };
    if !scheduled(&refreshed, &prepare) {
        journal.apply(
            core::ControllerInput::Scheduler(core::SchedulerInput::DispatchPrepare(
                sandbox, vm, slot,
            )),
            &mut NeverCrash,
        )?;
    }
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
    journal.apply(
        core::ControllerInput::Request(core::Request::Kill(request)),
        &mut NeverCrash,
    )?;
    finish_accepted_kill_v1(journal, sandbox, operation, worker)
}

/// Finish a previously accepted tenant kill or an atomic expiry kill. This
/// starts at the existing revocation intent and never accepts another kill.
///
/// # Errors
/// An uncertain host stop leaves the sandbox unreported as Destroyed.
pub fn finish_accepted_kill_v1<E, W>(
    journal: &mut ControllerJournal,
    sandbox: core::SandboxId,
    operation: core::OperationId,
    worker: &mut W,
) -> Result<core::World, JournalError>
where
    E: HostEvidence + Serialize + DeserializeOwned + PartialEq,
    W: FnMut(&WorkAction<core::EffectId>) -> Result<WorkReceipt<E>, WorkerTransportError>,
{
    let accepted = journal.world()?;
    let current = accepted
        .sandbox_record(sandbox)
        .and_then(|record| record.current_operation)
        .ok_or(JournalError::UnsupportedInput)?;
    if current.id != operation || current.kind != core::OperationKind::Kill {
        return Err(JournalError::UnsupportedInput);
    }
    drain(journal, sandbox, worker)?;
    let record = accepted
        .sandbox_record(sandbox)
        .ok_or(JournalError::CorruptStore)?;
    let key = core::OperationKey { sandbox, operation };
    let plan = accepted
        .operation_plan(key)
        .ok_or(JournalError::CorruptStore)?;
    if record.phase == core::Phase::Revoking {
        for vm in &plan.stop_vms {
            let stop = core::Command::StopVm {
                sandbox,
                epoch: record.epoch,
                generation: record.generation,
                vm: *vm,
            };
            if !scheduled(&journal.world()?, &stop) {
                journal.apply(
                    core::ControllerInput::Scheduler(core::SchedulerInput::StopVm(
                        sandbox,
                        record.generation,
                        *vm,
                    )),
                    &mut NeverCrash,
                )?;
            }
            drain(journal, sandbox, worker)?;
        }
        journal.apply(
            core::ControllerInput::Publication(core::PublicationInput::Destroyed(sandbox)),
            &mut NeverCrash,
        )?;
    } else if record.phase != core::Phase::Destroyed {
        return Err(JournalError::UnsupportedInput);
    }
    cleanup_destroyed_resources(journal, sandbox, record.generation, worker)
}

fn cleanup_destroyed_resources<E, W>(
    journal: &mut ControllerJournal,
    sandbox: core::SandboxId,
    generation: core::Generation,
    worker: &mut W,
) -> Result<core::World, JournalError>
where
    E: HostEvidence + Serialize + DeserializeOwned + PartialEq,
    W: FnMut(&WorkAction<core::EffectId>) -> Result<WorkReceipt<E>, WorkerTransportError>,
{
    let resources = journal.world()?.resource_index(sandbox);
    for vm in resources.vms {
        let world = journal.world()?;
        if world
            .vm_record(vm)
            .is_some_and(|vm_record| vm_record.status == core::VmStatus::Reclaimed)
        {
            continue;
        }
        if !scheduled(
            &world,
            &core::Command::CleanupVm {
                sandbox,
                generation,
                vm,
            },
        ) {
            journal.apply(
                core::ControllerInput::Scheduler(core::SchedulerInput::CleanupVm(
                    sandbox, generation, vm,
                )),
                &mut NeverCrash,
            )?;
        }
        drain(journal, sandbox, worker)?;
        if journal
            .world()?
            .vm_record(vm)
            .is_none_or(|vm_record| vm_record.status != core::VmStatus::Reclaimed)
        {
            return Err(JournalError::UnresolvedAction);
        }
    }
    for slot in resources.slots {
        let world = journal.world()?;
        if world
            .slot_record(slot)
            .is_some_and(|slot_record| slot_record.reclaimed)
        {
            continue;
        }
        if !scheduled(
            &world,
            &core::Command::CleanupSlot {
                sandbox,
                generation,
                slot,
            },
        ) {
            journal.apply(
                core::ControllerInput::Scheduler(core::SchedulerInput::CleanupSlot(
                    sandbox, generation, slot,
                )),
                &mut NeverCrash,
            )?;
        }
        drain(journal, sandbox, worker)?;
        if journal
            .world()?
            .slot_record(slot)
            .is_none_or(|slot_record| !slot_record.reclaimed)
        {
            return Err(JournalError::UnresolvedAction);
        }
    }
    journal.world()
}
