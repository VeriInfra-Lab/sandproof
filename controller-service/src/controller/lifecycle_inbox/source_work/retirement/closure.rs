//! Material eligibility and frozen closure bindings. Neither current epoch nor
//! current-generation connections replace the old execution's actual release.
use super::{
    Connection, ContentDigest, JournalError, MaterialRetirementClosure, MaterialRetirementReason,
    SourceWorkRecord, checkpoint, core, full, release,
};

fn restore_binding(record: &SourceWorkRecord) -> Result<Option<ContentDigest>, JournalError> {
    record
        .restore
        .as_ref()
        .map(|claim| {
            Ok(super::super::digest(&serde_json::to_vec(&(
                &claim.intent,
                claim.local,
                claim.generation,
                &claim.activation,
            ))?))
        })
        .transpose()
}

// Preserve the existing native terminal-cleanup contract: destruction may
// finish Kill, or a failed original operation's complete compensation.
fn terminal_result(ledger: &core::OperationLedgerRecord) -> bool {
    match (
        ledger.operation.kind,
        ledger.operation.status,
        ledger.result,
    ) {
        (
            core::OperationKind::Kill,
            core::OperationStatus::Succeeded,
            Some(core::OperationResult::Destroyed),
        ) => true,
        (
            core::OperationKind::Create | core::OperationKind::Pause | core::OperationKind::Resume,
            core::OperationStatus::Failed(expected),
            Some(core::OperationResult::Failed(observed)),
        ) => expected == observed,
        _ => false,
    }
}

fn bindings(
    record: &SourceWorkRecord,
    reason: MaterialRetirementReason,
) -> Result<MaterialRetirementClosure, JournalError> {
    Ok(MaterialRetirementClosure {
        release_binding: super::super::digest(&serde_json::to_vec(&record.material_release)?),
        restore_binding: restore_binding(record)?,
        reason,
    })
}

fn successor_matches(
    record: &SourceWorkRecord,
    successor: &SourceWorkRecord,
    world: &core::World,
) -> Result<bool, JournalError> {
    let Some(claim) = &record.restore else {
        return Ok(false);
    };
    let Some(capture) = &successor.capture else {
        return Ok(false);
    };
    let key = core::OperationKey {
        sandbox: successor.work.anchor.local.sandbox,
        operation: capture.operation,
    };
    let Some(plan) = world.operation_plan(key) else {
        return Err(JournalError::CorruptStore);
    };
    let Some(operation) = world.operation_record(key) else {
        return Err(JournalError::CorruptStore);
    };
    let Some(consumer) = record.material_release.consumer else {
        return Ok(false);
    };
    Ok(successor.work.anchor.execution_origin == claim.intent
        && successor.work.anchor.local.sandbox == claim.local.sandbox
        && successor.work.anchor.local.owner == claim.local.owner
        && successor.work.anchor.native.generation == claim.generation
        && successor.work.anchor.native.current_vm == Some(consumer.context.binding.vm)
        && successor.work.request().request.binding.session
            == record.work.request().request.binding.session
        && capture.completed.is_some()
        && capture.material_bytes.is_some()
        && plan
            .capture
            .is_some_and(|value| value.commit() == core::capture::CaptureCommit::Registered)
        && plan.target_snapshot.is_some()
        && operation.operation.kind == core::OperationKind::Pause
        && operation.operation.status == core::OperationStatus::Succeeded
        && operation.result == plan.target_snapshot.map(core::OperationResult::Paused)
        && full::material_writer_closed(successor, world)?)
}

pub(super) fn eligible(
    connection: &Connection,
    world: &core::World,
    records: &[SourceWorkRecord],
    record: &SourceWorkRecord,
) -> Result<Option<MaterialRetirementClosure>, JournalError> {
    if !release::complete(&record.material_release, record.restore.is_some())
        || !full::material_writer_closed(record, world)?
    {
        return Ok(None);
    }
    let sandbox = record.work.anchor.local.sandbox;
    let row = world
        .sandbox_record(sandbox)
        .ok_or(JournalError::CorruptStore)?;
    if row.terminal && row.phase == core::Phase::Destroyed {
        if core::maintenance::retirement::validate_terminal_cleanup(world, sandbox).is_err()
            || !checkpoint::material_retirement_pins_closed(connection, world, sandbox)?
        {
            return Ok(None);
        }
        return bindings(
            record,
            MaterialRetirementReason::Terminal {
                operation: core::OperationKey {
                    sandbox,
                    operation: row.current_operation.ok_or(JournalError::CorruptStore)?.id,
                },
                epoch: row.epoch,
                workspace: row.workspace,
            },
        )
        .map(Some);
    }
    for successor in records {
        if successor_matches(record, successor, world)? {
            let capture = successor
                .capture
                .as_ref()
                .ok_or(JournalError::CorruptStore)?;
            return bindings(
                record,
                MaterialRetirementReason::Superseded {
                    successor_work_binding: successor.work.requirement.work_binding,
                    successor_full_binding: capture.full.binding_digest()?,
                    manifest_digest: capture.completed.ok_or(JournalError::CorruptStore)?,
                },
            )
            .map(Some);
        }
    }
    Ok(None)
}

pub(super) fn validate_bindings(
    record: &SourceWorkRecord,
    closed: &MaterialRetirementClosure,
) -> Result<(), JournalError> {
    if !release::complete(&record.material_release, record.restore.is_some())
        || closed.release_binding
            != super::super::digest(&serde_json::to_vec(&record.material_release)?)
        || closed.restore_binding != restore_binding(record)?
    {
        return Err(JournalError::CorruptStore);
    }
    Ok(())
}

pub(super) fn validate(
    record: &SourceWorkRecord,
    world: &core::World,
    records: &[SourceWorkRecord],
    closed: &MaterialRetirementClosure,
) -> Result<(), JournalError> {
    validate_bindings(record, closed)?;
    match &closed.reason {
        MaterialRetirementReason::Terminal {
            operation,
            epoch,
            workspace,
        } => {
            let row = world
                .sandbox_record(operation.sandbox)
                .ok_or(JournalError::CorruptStore)?;
            let ledger = world
                .operation_record(*operation)
                .ok_or(JournalError::CorruptStore)?;
            let original = world
                .operation_plan(*operation)
                .ok_or(JournalError::CorruptStore)?;
            if operation.sandbox != record.work.anchor.local.sandbox
                || row.owner != record.work.anchor.local.owner
                || !row.terminal
                || row.phase != core::Phase::Destroyed
                || row.workspace != *workspace
                || original.epoch != *epoch
                || !terminal_result(ledger)
            {
                return Err(JournalError::CorruptStore);
            }
        }
        MaterialRetirementReason::Superseded {
            successor_work_binding,
            successor_full_binding,
            manifest_digest,
        } => {
            let successor = records
                .iter()
                .find(|value| value.work.requirement.work_binding == *successor_work_binding)
                .ok_or(JournalError::CorruptStore)?;
            let capture = successor
                .capture
                .as_ref()
                .ok_or(JournalError::CorruptStore)?;
            if !successor_matches(record, successor, world)?
                || capture.full.binding_digest()? != *successor_full_binding
                || capture.completed != Some(*manifest_digest)
            {
                return Err(JournalError::CorruptStore);
            }
        }
    }
    Ok(())
}
