use super::*;
use rusqlite::{Connection, params};
use sha2::{Digest, Sha256};

#[derive(Debug, PartialEq, Eq)]
struct Row {
    bytes: Vec<u8>,
    sha: Vec<u8>,
    key: Option<Vec<u8>>,
    reserve: i64,
}

fn row(connection: &Connection, id: i64) -> Row {
    connection
        .query_row(
            "SELECT state_json,sha256,work_key,reserve_bytes FROM controller_source_work_records WHERE id=?1",
            [id],
            |row| {
                Ok(Row {
                    bytes: row.get(0)?,
                    sha: row.get(1)?,
                    key: row.get(2)?,
                    reserve: row.get(3)?,
                })
            },
        )
        .unwrap()
}

fn assert_payload(row: &Row, record: &SourceWorkRecord, reserve: i64) {
    let bytes = serde_json::to_vec(record).unwrap();
    let sha: [u8; 32] = Sha256::digest(&bytes).into();
    assert_eq!(
        row.key.as_deref(),
        Some(record.work.requirement.work_binding.as_bytes().as_slice())
    );
    assert_eq!(row.bytes, bytes);
    assert_eq!(row.sha, sha);
    assert_eq!(row.reserve, reserve);
}

#[test]
fn retains_existing_slot_then_uses_minimum_vacancy_for_an_absent_key() {
    let dir = tempfile::tempdir().unwrap();
    let (mut journal, _) = ready(&dir.path().join("row-writer.db"));
    let original = prepare(&mut journal);
    let tx = journal.journal.connection.unchecked_transaction().unwrap();
    let occupied = row(&tx, 1);
    let empty = row(&tx, 3);
    assert_payload(&occupied, &original, empty.reserve);
    assert!(empty.key.is_none());
    // Swap two fixture IDs before the save, preserving row count and reservations.
    for (new, old) in [(i64::MAX, 3), (3, 1), (1, i64::MAX)] {
        assert_eq!(
            tx.execute(
                "UPDATE controller_source_work_records SET id=?1 WHERE id=?2",
                [new, old],
            )
            .unwrap(),
            1
        );
    }
    let other_empty = row(&tx, 2);
    let mut candidate = codec::load(&tx).unwrap().unwrap();
    assert_eq!(
        candidate.records.as_slice(),
        std::slice::from_ref(&original)
    );
    candidate.records[0].observation = Some(observation(original.work.requirement()));
    codec::save(&tx, &candidate, candidate.max_sessions).unwrap();
    assert_eq!(row(&tx, 1), empty);
    assert_eq!(row(&tx, 2), other_empty);
    assert_payload(&row(&tx, 3), &candidate.records[0], occupied.reserve);

    assert_eq!(tx.execute(
        "UPDATE controller_source_work_records SET state_json=?1,sha256=?2,work_key=NULL WHERE id=3",
        params![&empty.bytes, &empty.sha],
    ).unwrap(), 1);
    assert!(codec::load(&tx).unwrap().unwrap().records.is_empty());
    codec::save(&tx, &candidate, candidate.max_sessions).unwrap();
    assert_payload(&row(&tx, 1), &candidate.records[0], empty.reserve);
    assert_eq!(row(&tx, 2), other_empty);
    assert_eq!(row(&tx, 3), empty);
    assert_eq!(codec::load(&tx).unwrap().unwrap(), candidate);
}

#[test]
fn rejects_zero_updates_without_reporting_a_saved_payload() {
    let dir = tempfile::tempdir().unwrap();
    let (mut journal, _) = ready(&dir.path().join("row-writer-zero.db"));
    let original = prepare(&mut journal);
    let tx = journal.journal.connection.unchecked_transaction().unwrap();
    let before = row(&tx, 1);
    let vacant = row(&tx, 2);
    let mut candidate = codec::load(&tx).unwrap().unwrap();
    candidate.records[0].observation = Some(observation(original.work.requirement()));
    tx.execute_batch(
        "CREATE TEMP TRIGGER hold_source_write BEFORE UPDATE ON controller_source_work_records WHEN OLD.id=1 BEGIN SELECT RAISE(IGNORE); END;",
    ).unwrap();

    let result = codec::save(&tx, &candidate, candidate.max_sessions);
    assert_eq!(row(&tx, 1), before);
    assert_eq!(row(&tx, 2), vacant);
    assert!(
        matches!(result, Err(JournalError::CorruptStore)),
        "zero updated source rows must be rejected: {result:?}"
    );

    tx.execute_batch("DROP TRIGGER hold_source_write").unwrap();
    codec::save(&tx, &candidate, candidate.max_sessions).unwrap();
    assert_payload(&row(&tx, 1), &candidate.records[0], before.reserve);
    assert_eq!(row(&tx, 2), vacant);
    assert_eq!(codec::load(&tx).unwrap().unwrap(), candidate);
}
