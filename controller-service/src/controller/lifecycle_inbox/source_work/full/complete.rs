use super::{JournalError, WorkAction, codec, core, matches_action, save};
use crate::HostEvidence;
use sandproof_protocol::{WorkOutcome, WorkReceipt};

// Called by the original complete() before applying the same receipt in the
// same SQL transaction. There is no separate completion or shadow outbox.
pub(in crate::controller) fn save_receipt<E: HostEvidence>(
    connection: &rusqlite::Connection,
    world: &core::World,
    action: &WorkAction<core::EffectId>,
    receipt: &WorkReceipt<E>,
) -> Result<(), JournalError> {
    let Some(mut sources) = codec::load(connection)? else {
        return Ok(());
    };
    if !matches!(
        action.command.command(),
        core::Command::SealSnapshot { .. } | core::Command::CommitSnapshot { .. }
    ) {
        return Ok(());
    }
    let Some(record) = sources
        .records
        .iter_mut()
        .find(|r| matches_action(r, action, world))
    else {
        return Err(JournalError::Conflict);
    };
    if !matches_action(record, action, world) {
        return Err(JournalError::Conflict);
    }
    let capture = record.capture.as_mut().ok_or(JournalError::Conflict)?;
    let WorkOutcome::ObservedSuccess(evidence) = &receipt.outcome else {
        return Ok(());
    };
    let manifest = evidence
        .capture_manifest_digest(action.command.command())
        .ok_or(JournalError::InvalidHostEvidence)?;
    let bytes = evidence
        .capture_material_bytes(action.command.command())
        .filter(|n| *n > 0 && *n <= capture.reserved_material_bytes)
        .ok_or(JournalError::InvalidHostEvidence)?;
    if !evidence.validates(action.command.command()) {
        return Err(JournalError::InvalidHostEvidence);
    }
    if matches!(action.command.command(), core::Command::SealSnapshot { .. }) {
        if capture.action.as_ref() != Some(action)
            || capture.worker.is_none()
            || capture.completed.is_some_and(|old| old != manifest)
        {
            return Err(JournalError::Conflict);
        }
        capture.completed = Some(manifest);
        capture.material_bytes = Some(bytes);
    } else if capture.completed != Some(manifest) || capture.material_bytes != Some(bytes) {
        return Err(JournalError::InvalidHostEvidence);
    }
    save(connection, &sources)
}
