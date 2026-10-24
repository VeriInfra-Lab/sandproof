//! Reconstruct one exact observation question from original durable facts.
use super::{JournalError, SourceWorkRecord, core, key};
use crate::{
    HostSlotContext,
    capture::{TerminatedCaptureObservation, TerminatedCaptureWorkRequest},
    controller::{bounded, identity},
};
use core::maintenance::slots::HostSlotPhase;
use rusqlite::Connection;
use sandproof_protocol::{PROTOCOL_VERSION, WorkAction};

fn original_identity(
    record: &SourceWorkRecord,
) -> Result<core::capture::OriginalContinueIdentity, JournalError> {
    let capture = record.capture.as_ref().ok_or(JournalError::CorruptStore)?;
    super::super::validate::full_binding(record, &capture.full)?;
    let action = capture
        .action
        .as_ref()
        .ok_or(JournalError::UnresolvedAction)?;
    let core::Command::SealSnapshot {
        sandbox,
        epoch,
        generation,
        vm,
        ..
    } = *action.command.command()
    else {
        return Err(JournalError::CorruptStore);
    };
    if sandbox != record.work.anchor.local.sandbox
        || generation != record.work.anchor.native.generation
        || Some(vm) != record.work.anchor.native.current_vm
        || action.identity.epoch != epoch
        || action.identity.generation != generation
        || action.identity.attempt != action.command.attempt
    {
        return Err(JournalError::CorruptStore);
    }
    Ok(core::capture::OriginalContinueIdentity {
        key: key(record).ok_or(JournalError::CorruptStore)?,
        epoch,
        generation,
        vm,
    })
}

fn assemble(
    record: &SourceWorkRecord,
    stop: core::StopCoverage,
    stop_action: &WorkAction<core::EffectId>,
    host_slot: HostSlotContext,
) -> Result<TerminatedCaptureWorkRequest, JournalError> {
    let id = original_identity(record)?;
    let capture = record.capture.as_ref().ok_or(JournalError::CorruptStore)?;
    let seal_action = capture.action.as_ref().ok_or(JournalError::CorruptStore)?;
    let command = core::Command::StopVm {
        sandbox: id.key.sandbox,
        epoch: stop.epoch,
        generation: id.generation,
        vm: id.vm,
    };
    if *stop_action.command.command() != command
        || stop.operation.sandbox != id.key.sandbox
        || stop.operation == id.key
        || stop.epoch <= id.epoch
        || stop_action.command.attempt != stop.attempt
        || stop_action.identity.attempt != stop.attempt
        || stop_action.identity.epoch != stop.epoch
        || stop_action.identity.generation != id.generation
        || stop_action.identity.owner != seal_action.identity.owner
        || stop_action.identity.sandbox != seal_action.identity.sandbox
        || host_slot.binding.owner != record.work.anchor.local.owner
        || host_slot.binding.sandbox != id.key.sandbox
        || host_slot.binding.generation != id.generation
        || host_slot.binding.vm != id.vm
    {
        return Err(JournalError::CorruptStore);
    }
    Ok(TerminatedCaptureWorkRequest::from_source(
        id,
        seal_action.clone(),
        capture.full.clone(),
        record.work.requirement().clone(),
        capture.worker.clone(),
        (stop_action.clone(), stop),
        host_slot,
    ))
}

pub(super) fn from_saved(
    record: &SourceWorkRecord,
    stop: core::StopCoverage,
    observation: &TerminatedCaptureObservation,
) -> Result<TerminatedCaptureWorkRequest, JournalError> {
    let TerminatedCaptureObservation::StoppedAndDrained {
        stop_action,
        host_slot,
        helper_gate_inode,
        ..
    } = observation;
    if *helper_gate_inode == 0 {
        return Err(JournalError::CorruptStore);
    }
    assemble(record, stop, stop_action, *host_slot)
}

fn terminal_key(
    world: &core::World,
    record: &SourceWorkRecord,
) -> Result<core::OperationKey, JournalError> {
    let id = original_identity(record)?;
    let row = world
        .sandbox_record(id.key.sandbox)
        .ok_or(JournalError::CorruptStore)?;
    let operation = row
        .current_operation
        .ok_or(JournalError::UnresolvedAction)?;
    let key = core::OperationKey {
        sandbox: id.key.sandbox,
        operation: operation.id,
    };
    let ledger = world
        .operation_record(key)
        .ok_or(JournalError::CorruptStore)?;
    let plan = world
        .operation_plan(key)
        .ok_or(JournalError::CorruptStore)?;
    if !row.terminal
        || !matches!(row.phase, core::Phase::Revoking | core::Phase::Destroyed)
        || row.desired != core::Authority::Closed
        || row.effective != core::Authority::Closed
        || row.owner != record.work.anchor.local.owner
        || row.generation != id.generation
        || row.epoch <= id.epoch
        || operation.kind != core::OperationKind::Kill
        || operation.epoch != row.epoch
        || ledger.operation != operation
        || ledger.input
            != (core::OperationInput::Kill {
                generation: id.generation,
            })
        || plan.key != key
        || plan.epoch != row.epoch
        || plan.generation != id.generation
        || !plan.sealed
        || !plan.stop_vms.contains(&id.vm)
        || world.vm_record(id.vm).is_none_or(|vm| {
            vm.owner != row.owner
                || vm.sandbox != id.key.sandbox
                || vm.generation != id.generation
                || vm.status != core::VmStatus::Stopped
        })
    {
        return Err(JournalError::UnresolvedAction);
    }
    Ok(key)
}

fn successful_stop(
    connection: &Connection,
    namespace: &str,
    world: &core::World,
    record: &SourceWorkRecord,
) -> Result<(WorkAction<core::EffectId>, core::StopCoverage), JournalError> {
    let id = original_identity(record)?;
    let key = terminal_key(world, record)?;
    let plan = world
        .operation_plan(key)
        .ok_or(JournalError::CorruptStore)?;
    let command = core::Command::StopVm {
        sandbox: id.key.sandbox,
        epoch: plan.epoch,
        generation: id.generation,
        vm: id.vm,
    };
    let logical_id = core::LogicalEffectId {
        command: command.clone(),
    };
    let logical = world
        .logical_effect(&logical_id)
        .ok_or(JournalError::UnresolvedAction)?;
    let Some(core::LogicalEffectResult::Succeeded { attempt, result }) = logical.result else {
        return Err(JournalError::UnresolvedAction);
    };
    let effect = core::EffectId {
        logical: logical_id,
        attempt,
    };
    if logical.origin != core::EffectOrigin::Operation(key)
        || logical.current_attempt != attempt
        || world.effect_status(&effect)
            != Some(core::EffectStatus::Acknowledged(
                core::EffectOutcome::Succeeded(result),
            ))
        || !core::effect::effect_succeeded_check(
            world,
            &core::Command::InstallFence {
                sandbox: key.sandbox,
                epoch: plan.epoch,
                terminal: true,
            },
        )
    {
        return Err(JournalError::UnresolvedAction);
    }
    let public_sandbox = identity::sandbox_id(namespace, key.sandbox)?;
    let mut statement = connection.prepare(
        "SELECT action_json FROM outbox WHERE sandbox_id=?1 AND dispatch_state='receipted' ORDER BY id",
    )?;
    for row in statement.query_map([public_sandbox.as_str()], |r| r.get::<_, Vec<u8>>(0))? {
        let action: WorkAction<core::EffectId> = serde_json::from_slice(&row?)?;
        if action.command != effect {
            continue;
        }
        if action.identity.protocol_version != PROTOCOL_VERSION
            || action.identity.owner != identity::owner_id(record.work.anchor.local.owner)?
            || action.identity.sandbox != public_sandbox
            || action.identity.operation != identity::operation_id(namespace, key)?
            || action.identity.epoch != plan.epoch
            || action.identity.generation != id.generation
            || action.identity.attempt != attempt
            || action.identity.resource != identity::resource(namespace, &command)?
        {
            return Err(JournalError::CorruptStore);
        }
        return Ok((
            action,
            core::StopCoverage {
                operation: key,
                epoch: plan.epoch,
                attempt,
                result,
            },
        ));
    }
    Err(JournalError::UnresolvedAction)
}

pub(super) fn current(
    connection: &Connection,
    namespace: &str,
    world: &core::World,
    record: &SourceWorkRecord,
) -> Result<TerminatedCaptureWorkRequest, JournalError> {
    super::super::validate::validate_record(record, world)?;
    let id = original_identity(record)?;
    let capture = record.capture.as_ref().ok_or(JournalError::CorruptStore)?;
    let handoff = world.operation_plan(id.key).and_then(|p|p.capture)
        .ok_or(JournalError::CorruptStore)?;
    // The original Continue receipt may already be referenced by immutable
    // native history. It already closes the writer; never replace that End.
    if handoff.original_continuation().is_some() {
        return Err(JournalError::Conflict);
    }
    let seal = capture
        .action
        .as_ref()
        .ok_or(JournalError::UnresolvedAction)?;
    if capture.completed.is_none() && !matches!(world.effect_status(&seal.command),
        Some(core::EffectStatus::Dispatched | core::EffectStatus::OutcomeUnknown
            | core::EffectStatus::RetiredByCaptureDrain(_)))
    { return Err(JournalError::UnresolvedAction); }
    if seal.identity.owner != identity::owner_id(record.work.anchor.local.owner)?
        || seal.identity.sandbox != identity::sandbox_id(namespace, id.key.sandbox)?
        || seal.identity.operation != identity::operation_id(namespace, id.key)?
        || seal.identity.resource != identity::resource(namespace, seal.command.command())?
    {
        return Err(JournalError::CorruptStore);
    }
    let (action, stop) = successful_stop(connection, namespace, world, record)?;
    let config = bounded::config(connection)?.ok_or(JournalError::UnsupportedStore)?;
    let host_slot = bounded::validate_contexts(connection, world, &config)?
        .into_iter()
        .find(|c| c.binding.sandbox == id.key.sandbox)
        .ok_or(JournalError::UnresolvedAction)?;
    if world
        .maintenance_state()
        .and_then(|s| s.host_slot(host_slot.position))
        .is_none_or(|s| s.binding != host_slot.binding || s.phase != HostSlotPhase::Occupied)
    {
        return Err(JournalError::UnresolvedAction);
    }
    assemble(record, stop, &action, host_slot)
}
