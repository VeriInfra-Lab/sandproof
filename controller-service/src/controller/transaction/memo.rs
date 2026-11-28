//! Original transaction decisions with an explicit mutable cell interface.
//! Cells performs only the supplied value transfer; this core owns the order,
//! invalidation decision and commit-error classification. It never runs SQL or
//! authorizes a host effect, and does not assume an error means a rollback.
use crate::JournalError;

pub(super) trait Cells<W> {
    fn set_committed(&mut self, value: Option<W>);
    fn take_candidate(&mut self) -> Option<W>;
}

pub(super) fn commit_with<W, T, S, F>(
    transaction: &mut Option<T>,
    cells: &mut S,
    commit: F,
) -> Result<(), JournalError>
where
    S: Cells<W>,
    F: FnOnce(T) -> rusqlite::Result<()>,
{
    // Clear before even taking the transaction: a panic or failed commit may
    // leave an unknown outcome. Only a confirmed return can publish a value.
    cells.set_committed(None);
    commit(transaction.take().expect("live transaction")).map_err(JournalError::CommitUncertain)?;
    let candidate = cells.take_candidate();
    cells.set_committed(candidate);
    Ok(())
}

pub(super) fn rollback_if_live<W, T, S, R>(transaction: &mut Option<T>, cells: &mut S, rollback: R)
where
    S: Cells<W>,
    R: FnOnce(T) -> rusqlite::Result<()>,
{
    if let Some(transaction) = transaction.take()
        && rollback(transaction).is_err()
    {
        // Drop cannot report rollback failure; retain no potentially stale memo.
        cells.set_committed(None);
    }
}
