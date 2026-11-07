//! Forward one library call through a mutable borrow of its original raw
//! transaction. SQL selection, encoding and result checks stay with the caller.
use rusqlite::{
    Transaction,
    types::{ToSqlOutput, ValueRef},
};

pub(super) fn execute_blobs(
    transaction: &mut Transaction<'_>,
    statement: &str,
    bytes: &[u8],
    digest: &[u8],
) -> rusqlite::Result<usize> {
    transaction.execute(
        statement,
        [
            ToSqlOutput::Borrowed(ValueRef::Blob(bytes)),
            ToSqlOutput::Borrowed(ValueRef::Blob(digest)),
        ],
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::{Connection, TransactionBehavior};

    const UPDATE: &str = "UPDATE controller_lifecycle_inbox SET state_json=?1,sha256=?2 WHERE id=1";
    const SELECT: &str = "SELECT state_json,sha256 FROM controller_lifecycle_inbox WHERE id=1";

    fn database() -> Connection {
        let connection = Connection::open_in_memory().unwrap();
        connection
            .execute_batch(
                "CREATE TABLE controller_lifecycle_inbox(id INTEGER PRIMARY KEY,\
                 state_json BLOB NOT NULL,sha256 BLOB NOT NULL);\
                 INSERT INTO controller_lifecycle_inbox VALUES(1,X'00',X'01');",
            )
            .unwrap();
        connection
    }

    fn read(connection: &Connection) -> (Vec<u8>, Vec<u8>) {
        connection
            .query_row(SELECT, [], |row| Ok((row.get(0)?, row.get(1)?)))
            .unwrap()
    }

    #[test]
    fn write_is_observed_by_the_same_transaction_before_commit() {
        let mut connection = database();
        let bytes = [0, 255, 128, 0, 42];
        let digest = [17; 32];
        let mut transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .unwrap();
        assert_eq!(
            execute_blobs(&mut transaction, UPDATE, &bytes, &digest).unwrap(),
            1
        );
        assert_eq!(read(&transaction), (bytes.to_vec(), digest.to_vec()));
        transaction.commit().unwrap();
        assert_eq!(read(&connection), (bytes.to_vec(), digest.to_vec()));
    }

    #[test]
    fn normal_drop_rolls_back_the_uncommitted_write() {
        let mut connection = database();
        let original = read(&connection);
        {
            let mut transaction = connection
                .transaction_with_behavior(TransactionBehavior::Immediate)
                .unwrap();
            execute_blobs(&mut transaction, UPDATE, &[2, 3], &[4; 32]).unwrap();
            assert_eq!(read(&transaction), (vec![2, 3], vec![4; 32]));
        }
        assert_eq!(read(&connection), original);
    }

    #[test]
    fn library_error_does_not_imply_an_unchanged_transaction_view() {
        let mut connection = database();
        // A primitive-level fault fixture, not a supported production schema:
        // SQLite FAIL keeps the preceding row change in the live transaction.
        connection
            .execute_batch(
                "CREATE TRIGGER fail_after_update AFTER UPDATE ON controller_lifecycle_inbox \
                 BEGIN SELECT RAISE(FAIL,'after row change'); END;",
            )
            .unwrap();
        let original = read(&connection);
        let mut transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .unwrap();
        assert!(execute_blobs(&mut transaction, UPDATE, &[9, 0], &[8; 32]).is_err());
        assert_eq!(read(&transaction), (vec![9, 0], vec![8; 32]));
        transaction.rollback().unwrap();
        assert_eq!(read(&connection), original);
    }
}
