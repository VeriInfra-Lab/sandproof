use super::{AcceptedWrite, JournalError, as_sql_integer};
use rusqlite::Transaction;

pub(super) fn next_revision(expected_revision: Option<u64>) -> Result<(u64, i64), JournalError> {
    let next_revision = match expected_revision {
        Some(revision) => revision
            .checked_add(1)
            .ok_or(JournalError::RevisionConflict)?,
        None => 1,
    };
    let next_revision_sql = as_sql_integer(next_revision)?;
    Ok((next_revision, next_revision_sql))
}

pub(super) fn update_existing<C>(
    transaction: &Transaction<'_>,
    write: &AcceptedWrite<C>,
    next_revision_sql: i64,
    expected: u64,
) -> Result<(), JournalError> {
    let changed = transaction.execute(
        "UPDATE sandboxes SET revision=?1, state_blob=?2
             WHERE sandbox_id=?3 AND owner_id=?4 AND revision=?5",
        (
            next_revision_sql,
            &write.sandbox_state,
            write.sandbox.as_str(),
            write.owner.as_str(),
            as_sql_integer(expected)?,
        ),
    )?;
    if changed != 1 {
        return Err(JournalError::RevisionConflict);
    }
    Ok(())
}
