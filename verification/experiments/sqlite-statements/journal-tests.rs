#[cfg(test)]
mod sqlite_statements_trial {
    use super::*;
    use crate::NeverCrash;

    fn write(
        owner: &str,
        sandbox: &str,
        operation: &str,
        revision: Option<u64>,
    ) -> AcceptedWrite<()> {
        AcceptedWrite {
            owner: TenantId::try_from(owner).unwrap(),
            sandbox: SandboxId::try_from(sandbox).unwrap(),
            operation: OperationId::try_from(operation).unwrap(),
            expected_revision: revision,
            sandbox_state: vec![0, 255, 17, 0],
            operation_state: vec![41, 0, 254],
            actions: Vec::new(),
        }
    }

    fn fixture() -> (tempfile::TempDir, Journal) {
        assert_eq!(rusqlite::version_number(), 3_053_002);
        let directory = tempfile::TempDir::new().unwrap();
        let mut journal = Journal::open(directory.path().join("statements.sqlite3")).unwrap();
        journal
            .commit(
                &write("tenant-a", "sandbox-a", "operation-a", None),
                &mut NeverCrash,
            )
            .unwrap();
        journal
            .commit(
                &write("tenant-b", "sandbox-b", "operation-b", None),
                &mut NeverCrash,
            )
            .unwrap();
        (directory, journal)
    }

    type StoredRow = (String, String, i64, Vec<u8>);

    fn rows(journal: &Journal) -> Vec<StoredRow> {
        let mut statement = journal.connection.prepare(
            "SELECT sandbox_id, owner_id, revision, state_blob FROM sandboxes ORDER BY sandbox_id",
        ).unwrap();
        statement
            .query_map([], |row| {
                Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
            })
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap()
    }

    #[test]
    fn exact_update_changes_original_row_and_preserves_other_row() {
        let (_directory, mut journal) = fixture();
        let before = rows(&journal);
        let mut next = write("tenant-a", "sandbox-a", "operation-a", Some(1));
        next.sandbox_state = vec![127, 0, 255, 19, 0];
        assert_eq!(journal.commit(&next, &mut NeverCrash).unwrap(), 2);
        let after = rows(&journal);
        assert_eq!(after.len(), before.len());
        assert_eq!(
            after[0],
            (
                "sandbox-a".to_owned(),
                "tenant-a".to_owned(),
                2,
                next.sandbox_state.clone()
            )
        );
        assert_eq!(after[1], before[1]);
        let observed = journal.sandbox(&next.sandbox).unwrap().unwrap();
        assert_eq!(observed.owner, next.owner);
        assert_eq!(observed.revision, 2);
        assert_eq!(observed.state, next.sandbox_state);
    }

    #[test]
    fn wrong_owner_never_succeeds_or_changes_any_row() {
        let (_directory, mut journal) = fixture();
        let before = rows(&journal);
        let next = write("tenant-wrong", "sandbox-a", "operation-a", Some(1));
        assert!(
            matches!(
                journal.commit(&next, &mut NeverCrash),
                Err(JournalError::RevisionConflict)
            ),
            "original SQL owner mismatch must reject before subsequent writes"
        );
        assert_eq!(rows(&journal), before);
    }

    #[test]
    fn wrong_revision_never_succeeds_or_changes_any_row() {
        let (_directory, mut journal) = fixture();
        let before = rows(&journal);
        let mut next = write("tenant-a", "sandbox-a", "operation-a", Some(0));
        next.sandbox_state = vec![9, 8, 0, 255];
        assert!(
            matches!(
                journal.commit(&next, &mut NeverCrash),
                Err(JournalError::RevisionConflict)
            ),
            "original zero-row UPDATE must be a revision conflict"
        );
        assert_eq!(rows(&journal), before);
    }

    #[test]
    fn missing_target_never_becomes_a_successful_update() {
        let (_directory, mut journal) = fixture();
        let before = rows(&journal);
        let next = write("tenant-a", "sandbox-missing", "operation-missing", Some(1));
        assert!(matches!(
            journal.commit(&next, &mut NeverCrash),
            Err(JournalError::RevisionConflict)
        ));
        assert_eq!(rows(&journal), before);
        assert!(journal.sandbox(&next.sandbox).unwrap().is_none());
    }

    #[test]
    fn revision_overflow_and_sql_integer_limit_reject_before_update() {
        let (_directory, mut journal) = fixture();
        let before = rows(&journal);
        journal.connection.execute_batch(
            "CREATE TRIGGER forbid_update BEFORE UPDATE ON sandboxes BEGIN SELECT RAISE(ABORT, 'unexpected UPDATE'); END;",
        ).unwrap();
        let next = write("tenant-a", "sandbox-a", "operation-a", Some(u64::MAX));
        assert!(matches!(
            journal.commit(&next, &mut NeverCrash),
            Err(JournalError::RevisionConflict)
        ));
        let next = write(
            "tenant-a",
            "sandbox-a",
            "operation-a",
            Some(u64::try_from(i64::MAX).unwrap()),
        );
        assert!(matches!(
            journal.commit(&next, &mut NeverCrash),
            Err(JournalError::Conflict)
        ));
        assert_eq!(rows(&journal), before);
    }

    #[test]
    fn largest_representable_next_revision_roundtrips_actual_columns() {
        let (_directory, mut journal) = fixture();
        let previous = i64::MAX - 1;
        journal
            .connection
            .execute(
                "UPDATE sandboxes SET revision=?1 WHERE sandbox_id='sandbox-a'",
                [previous],
            )
            .unwrap();
        let next = write(
            "tenant-a",
            "sandbox-a",
            "operation-a",
            Some(u64::try_from(previous).unwrap()),
        );
        assert_eq!(
            journal.commit(&next, &mut NeverCrash).unwrap(),
            u64::try_from(i64::MAX).unwrap()
        );
        let observed = journal.sandbox(&next.sandbox).unwrap().unwrap();
        assert_eq!(observed.revision, u64::try_from(i64::MAX).unwrap());
        assert_eq!(observed.state, next.sandbox_state);
    }

    #[test]
    fn original_sql_error_stays_an_error_and_outer_transaction_rolls_back() {
        let (_directory, mut journal) = fixture();
        let before = rows(&journal);
        journal.connection.execute_batch(
            "CREATE TRIGGER fail_before_update BEFORE UPDATE ON sandboxes BEGIN SELECT RAISE(ABORT, 'controlled SQL error'); END;",
        ).unwrap();
        let next = write("tenant-a", "sandbox-a", "operation-a", Some(1));
        assert!(matches!(
            journal.commit(&next, &mut NeverCrash),
            Err(JournalError::Sqlite(_))
        ));
        assert_eq!(rows(&journal), before);
    }

    #[test]
    fn statement_error_can_leave_an_effect_inside_original_live_transaction() {
        let (_directory, mut journal) = fixture();
        let before = rows(&journal);
        journal.connection.execute_batch(
            "CREATE TRIGGER fail_after_update AFTER UPDATE ON sandboxes BEGIN SELECT RAISE(FAIL, 'controlled partial effect'); END;",
        ).unwrap();
        let mut next = write("tenant-a", "sandbox-a", "operation-a", Some(1));
        next.sandbox_state = vec![77, 0, 255];
        let transaction = journal.connection.transaction().unwrap();
        assert!(matches!(
            write_accepted_with_ids(&transaction, &next, None),
            Err(JournalError::Sqlite(_))
        ));
        let current: (i64, Vec<u8>) = transaction
            .query_row(
                "SELECT revision, state_blob FROM sandboxes WHERE sandbox_id='sandbox-a'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(current, (2, next.sandbox_state));
        transaction.rollback().unwrap();
        assert_eq!(rows(&journal), before);
    }
}
