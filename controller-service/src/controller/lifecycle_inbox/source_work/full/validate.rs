use super::*;
use sandproof_protocol::lifecycle::LifecycleOperation;

pub(super) fn full_binding(
    record: &SourceWorkRecord,
    full: &FullCaptureWorkRequest,
) -> Result<(), JournalError> {
    let work = &record.work;
    let LifecycleOperation::Unload {
        source_incarnation,
        capture,
    } = work.request().request.operation
    else {
        return Err(JournalError::Conflict);
    };
    if !full.valid()
        || full.pause.binding.operation != work.request().request.binding
        || full.pause.binding.source_incarnation != source_incarnation
        || full.pause.binding.capture != capture
        || full.pause.source_work_binding != work.requirement.work_binding
        || full.pause.node != work.requirement.node
        || full.pause.source_resource != work.requirement.resource
        || full.pause.source != work.requirement.source
    {
        return Err(JournalError::Conflict);
    }
    Ok(())
}
pub(in crate::controller) fn validate_record(
    record: &SourceWorkRecord,
    world: &core::World,
) -> Result<(), JournalError> {
    let Some(capture) = &record.capture else {
        return Ok(());
    };
    full_binding(record, &capture.full).map_err(|_| JournalError::CorruptStore)?;
    let key = key(record).ok_or(JournalError::CorruptStore)?;
    let plan = world
        .operation_plan(key)
        .ok_or(JournalError::CorruptStore)?;
    if plan.key != key
        || plan.generation != record.work.anchor.native.generation
        || plan.capture.is_none()
        || capture.reserved_material_bytes != reservation(&capture.full)?
        || capture.completed.is_some() != capture.material_bytes.is_some()
        || capture
            .material_bytes
            .is_some_and(|n| n == 0 || n > capture.reserved_material_bytes)
        || serde_json::to_vec(capture)?.len() > RECORD_BYTES
        || capture.worker.as_ref().is_some_and(|w| !w.valid())
        || (capture.worker.is_some() && capture.action.is_none())
        || (capture.completed.is_some() && capture.worker.is_none())
    {
        return Err(JournalError::CorruptStore);
    }
    if let Some(action) = &capture.action {
        if !matches_action(record, action, world)
            || !matches!(action.command.command(), core::Command::SealSnapshot { .. })
        {
            return Err(JournalError::CorruptStore);
        }
        if capture.completed.is_some()
            && !core::effect::effect_succeeded_check(world, action.command.command())
        {
            return Err(JournalError::CorruptStore);
        }
    }
    Ok(())
}
