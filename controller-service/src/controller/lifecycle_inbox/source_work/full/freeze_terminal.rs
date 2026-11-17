//! SQL authorization for the narrow terminal Freeze exception.
//! No launch permit/worker, no material result, and a durable closed Full gate.
use super::{JournalError, SourceWorkRecord, codec, core, end::CaptureEndRecord, key};
use rusqlite::Connection;

fn closed(record: &SourceWorkRecord) -> Result<bool, JournalError> {
    let Some(capture) = &record.capture else {
        return Ok(false);
    };
    super::end::validate_record(record)?;
    Ok(capture.action.is_none()
        && capture.worker.is_none()
        && capture.completed.is_none()
        && capture.material_bytes.is_none()
        && matches!(
            capture.end,
            Some(CaptureEndRecord::NeverReleased {
                seal_action: None,
                ..
            })
        ))
}

pub(in crate::controller) fn permits(
    connection: &Connection,
    world: &core::World,
    freeze: &core::EffectId,
) -> Result<bool, JournalError> {
    let core::Command::FreezeVm {
        sandbox,
        epoch,
        generation,
        vm,
    } = *freeze.command()
    else {
        return Ok(false);
    };
    let Some(logical) = world.logical_effect(&freeze.logical) else {
        return Ok(false);
    };
    let core::EffectOrigin::Operation(original) = logical.origin else {
        return Ok(false);
    };
    let Some(sources) = codec::load(connection)? else {
        return Ok(false);
    };
    for record in &sources.records {
        if key(record) != Some(original) {
            continue;
        }
        super::validate::validate_record(record, world)?;
        let plan = world
            .operation_plan(original)
            .ok_or(JournalError::CorruptStore)?;
        return Ok(record.work.anchor.local.sandbox == sandbox
            && record.work.anchor.native.generation == generation
            && record.work.anchor.native.current_vm == Some(vm)
            && plan.epoch == epoch
            && logical.current_attempt == freeze.attempt
            && closed(record)?);
    }
    Ok(false)
}

pub(in crate::controller) fn allows_command(
    connection: &Connection,
    world: &core::World,
    command: &core::Command,
) -> Result<bool, JournalError> {
    if !core::capture::freeze_terminal::allowed(world, command) {
        return Ok(false);
    }
    for id in world.effect_ids() {
        if id.command().sandbox() == command.sandbox()
            && core::capture::freeze_terminal::current_stop(world, id).is_some()
            && permits(connection, world, id)?
        {
            return Ok(true);
        }
    }
    Ok(false)
}
