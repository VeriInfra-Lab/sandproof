use super::*;

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
    let Some(index) = sources
        .records
        .iter()
        .position(|r| r.work.anchor.local.sandbox == sandbox)
    else {
        return Ok(true);
    };
    let record = &sources.records[index];
    let Some(capture) = &record.capture else {
        return Ok(!matches!(
            action.command.command(),
            core::Command::FreezeVm { .. }
                | core::Command::SealSnapshot { .. }
                | core::Command::CommitSnapshot { .. }
        ));
    };
    match action.command.command() {
        core::Command::FreezeVm { .. }
        | core::Command::SealSnapshot { .. }
        | core::Command::CommitSnapshot { .. } => {
            if !matches_action(record, action, world) {
                return Ok(false);
            }
        }
        core::Command::StopVm { .. } | core::Command::CleanupVm { .. } => {
            // Unknown Full may still write. Neither terminal intent nor a dead
            // manager/helper proves that this physical responsibility ended.
            if capture.action.is_some() && capture.completed.is_none() {
                return Ok(false);
            }
        }
        _ => (),
    }
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
