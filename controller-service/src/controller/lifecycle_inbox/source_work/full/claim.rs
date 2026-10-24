use super::{JournalError, MAX_FULL, WorkAction, codec, core, matches_action, save};

// Invoked in the original dispatch transaction before the ready->dispatched CAS.
pub(in crate::controller) fn claim(
    connection: &rusqlite::Connection,
    world: &core::World,
    action: &WorkAction<core::EffectId>,
) -> Result<bool, JournalError> {
    let Some(mut sources) = codec::load(connection)? else {
        return Ok(true);
    };
    let sandbox = action.command.command().sandbox();
    if matches!(
        action.command.command(),
        core::Command::StopVm { .. }
            | core::Command::CleanupVm { .. }
            | core::Command::CleanupSlot { .. }
    ) && sources.records.iter().any(|record| {
        record.work.anchor.local.sandbox == sandbox
            && record
                .capture
                .as_ref()
                .is_some_and(|capture| capture.action.is_some() && capture.completed.is_none())
    }) {
        // No unfinished writer is forgotten when a newer source is admitted.
        return Ok(false);
    }
    let capture_action = matches!(
        action.command.command(),
        core::Command::FreezeVm { .. }
            | core::Command::SealSnapshot { .. }
            | core::Command::CommitSnapshot { .. }
    );
    if !capture_action {
        return Ok(true);
    }
    let Some(index) = sources
        .records
        .iter()
        .position(|record| matches_action(record, action, world))
    else {
        return Ok(false);
    };
    let record = &sources.records[index];
    let capture = record.capture.as_ref().ok_or(JournalError::Conflict)?;
    if matches!(action.command.command(), core::Command::SealSnapshot { .. }) {
        let active = sources
            .records
            .iter()
            .filter(|r| {
                r.capture
                    .as_ref()
                    .is_some_and(|c| c.action.is_some() && c.completed.is_none())
            })
            .count();
        if capture.action.is_some() || active >= MAX_FULL {
            return Ok(false);
        }
        sources.records[index]
            .capture
            .as_mut()
            .ok_or(JournalError::CorruptStore)?
            .action = Some(action.clone());
        save(connection, &sources)?;
    }
    Ok(true)
}
