
// Extraction-only roots: unchanged argument forwarding into actual methods.
#[allow(dead_code)]
pub(super) fn sqlite_wrapper_probe<'a>(
    transaction: WorldTransaction<'a>,
    commit: impl FnOnce(Transaction<'a>) -> rusqlite::Result<()>,
) -> Result<(), JournalError> {
    transaction.commit_with(commit)
}

#[allow(dead_code)]
pub(super) fn sqlite_commit_entry_probe(
    transaction: WorldTransaction<'_>,
) -> Result<(), JournalError> {
    transaction.commit()
}
