//! Volatile replay memo under the controller's single-writer lease. Only a
//! confirmed commit publishes a candidate; the durable recovery image stays the oracle.
use super::storage;
use crate::JournalError;
use rusqlite::{Connection, Transaction, TransactionBehavior};
use sandbox_controller_core::World;
use std::{cell::RefCell, ops::Deref};

pub(super) struct CommittedWorld {
    value: RefCell<Option<World>>,
    // SQLite changes this connection-local value on another connection's
    // commit, including edits to old rows or SQL-only projections. It is not
    // a lease or an adversarial database-integrity/security mechanism.
    data_version: i64,
}

fn data_version(connection: &Connection) -> Result<i64, JournalError> {
    Ok(connection.query_row("PRAGMA data_version", [], |row| row.get(0))?)
}

impl CommittedWorld {
    pub(super) fn new(connection: &Connection, world: World) -> Result<Self, JournalError> {
        Ok(Self {
            value: RefCell::new(Some(world)),
            data_version: data_version(connection)?,
        })
    }

    pub(super) fn check_identity(&self, connection: &Connection) -> Result<(), JournalError> {
        match data_version(connection) {
            Ok(version) if version == self.data_version => Ok(()),
            result => {
                self.invalidate();
                result.and(Err(JournalError::CorruptStore))
            }
        }
    }

    pub(super) fn ensure_ready(&self, connection: &Connection) -> Result<(), JournalError> {
        if !connection.is_autocommit() {
            self.invalidate();
            return Err(JournalError::CorruptStore);
        }
        self.check_identity(connection)?;
        if self.value.borrow().is_none() {
            let world = storage::load_world(connection)?;
            self.check_identity(connection)?;
            *self.value.borrow_mut() = Some(world);
        }
        Ok(())
    }

    pub(super) fn world(&self, connection: &Connection) -> Result<World, JournalError> {
        self.ensure_ready(connection)?;
        self.value
            .borrow()
            .clone()
            .ok_or(JournalError::CorruptStore)
    }

    pub(super) fn invalidate(&self) {
        *self.value.borrow_mut() = None;
    }
}

pub(super) struct WorldTransaction<'a> {
    transaction: Option<Transaction<'a>>,
    committed: &'a CommittedWorld,
    candidate: RefCell<Option<World>>,
    pub(super) input_start: u64,
}

impl<'a> WorldTransaction<'a> {
    pub(super) fn begin(
        connection: &'a mut Connection,
        committed: &'a CommittedWorld,
    ) -> Result<Self, JournalError> {
        let world = super::diagnose_stage("transaction.world",committed.world(connection))?;
        super::diagnose_stage("transaction.prepare-write",super::checkpoint::prepare_write(connection, &world, committed))?;
        let transaction = super::diagnose_stage("transaction.begin-immediate",connection.transaction_with_behavior(TransactionBehavior::Immediate))?;
        let input_start = super::diagnose_stage("transaction.input-frontier",super::allocators::input_high_water(&transaction))?;
        let guard = Self {
            input_start,
            transaction: Some(transaction),
            committed,
            candidate: RefCell::new(Some(world)),
        };
        // Check again under BEGIN IMMEDIATE to close the external-writer gap.
        super::diagnose_stage("transaction.writer-identity",committed.check_identity(&guard))?;
        Ok(guard)
    }

    pub(super) fn world(&self) -> World {
        self.candidate
            .borrow()
            .as_ref()
            .expect("live candidate")
            .clone()
    }

    // Called only after storage::apply appended the accepted input and all
    // projections/actions. Every input in a multi-input transaction advances.
    pub(super) fn advance(&self, world: World) {
        *self.candidate.borrow_mut() = Some(world);
    }

    pub(super) fn commit(self) -> Result<(), JournalError> {
        super::diagnose_stage("commit.checkpoint",super::checkpoint::validate_candidate(&self, &self.world()))?;
        super::diagnose_stage("commit.lifecycle",super::lifecycle_inbox::validate(&self, &self.world()))?;
        super::diagnose_stage("commit.capacity",super::capacity::validate(&self, &self.world()))?;
        super::diagnose_stage("commit.input-tail",super::checkpoint::validate_transaction(&self))?;
        #[cfg(test)]
        if let Some(durable) = COMMIT_FAILURE.with(std::cell::Cell::take) {
            return self.commit_with(|transaction| {
                if durable {
                    transaction.commit()?;
                } else {
                    transaction.rollback()?;
                }
                Err(rusqlite::Error::InvalidQuery)
            });
        }
        self.commit_with(Transaction::commit)
    }

    fn commit_with(
        mut self,
        commit: impl FnOnce(Transaction<'a>) -> rusqlite::Result<()>,
    ) -> Result<(), JournalError> {
        // Invalidate before the call: even a failed commit/rollback may have
        // committed. No candidate is visible until success is confirmed.
        self.committed.invalidate();
        commit(self.transaction.take().expect("live transaction"))
            .map_err(JournalError::CommitUncertain)?;
        *self.committed.value.borrow_mut() = self.candidate.borrow_mut().take();
        Ok(())
    }
}

impl<'a> Deref for WorldTransaction<'a> {
    type Target = Transaction<'a>;

    fn deref(&self) -> &Self::Target {
        self.transaction.as_ref().expect("live transaction")
    }
}

impl Drop for WorldTransaction<'_> {
    fn drop(&mut self) {
        if let Some(transaction) = self.transaction.take()
            && transaction.rollback().is_err()
        {
            // The Drop path cannot report rollback failure to the caller.
            // Invalidation prevents any later use of a possibly stale memo.
            self.committed.invalidate();
        }
    }
}

// A test-only lost commit reply, with both durable outcomes. Production has
// no injectable branch and does not reinterpret post-commit crash hooks.
#[cfg(test)]
thread_local! {
    static COMMIT_FAILURE: std::cell::Cell<Option<bool>> = const { std::cell::Cell::new(None) };
}

#[cfg(test)]
pub(super) fn fail_next_commit(durable: bool) {
    COMMIT_FAILURE.with(|slot| {
        assert!(slot.replace(Some(durable)).is_none());
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn foreign_commit_between_sql_commit_and_publication_cannot_be_absorbed() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("controller.db");
        let mut journal =
            crate::controller::tests_support::open(&path, "0123456789abcdef").unwrap();
        let external = Connection::open(&path).unwrap();
        let tx = WorldTransaction::begin(&mut journal.journal.connection, &journal.committed_world)
            .unwrap();
        tx.execute("CREATE TABLE test_own_write(id INTEGER)", [])
            .unwrap();
        tx.commit_with(|tx| {
            tx.commit()?;
            external.execute(
                "UPDATE controller_meta SET namespace='fedcba9876543210'",
                [],
            )?;
            Ok(())
        })
        .unwrap();
        assert!(matches!(journal.world(), Err(JournalError::CorruptStore)));
        assert!(matches!(
            journal.has_template_registry(),
            Err(JournalError::CorruptStore)
        ));
    }
}
