
// Private Ning supplement, appended inside existing capture::tests only after review.
#[test]
fn owned_agreement_preserves_prefix_on_invalid_and_conflicting_observation() {
    let expected = source();
    let (result, slot) = agree(None, expected.clone());
    result.unwrap();
    assert_eq!(slot, Some(expected.clone()));
    let (result, slot) = agree(slot, expected.clone());
    result.unwrap();
    assert_eq!(slot, Some(expected.clone()));
    let mut different = expected.clone();
    different.boot_id.push_str("-different");
    let (result, slot) = agree(slot, different);
    assert!(matches!(result, Err(JournalError::UnresolvedAction)));
    assert_eq!(slot, Some(expected.clone()));
    let mut invalid = expected.clone();
    invalid.pid = 0;
    let (result, slot) = agree(slot, invalid);
    assert!(matches!(result, Err(JournalError::UnresolvedAction)));
    assert_eq!(slot, Some(expected));
}

#[test]
fn row_sql_error_preserves_both_prior_observations() {
    let dir = tempfile::tempdir().unwrap();
    let expected = source();
    let journal = running(&dir.path().join("controller.db"), Some(&expected), Some(&expected));
    let world = journal.world().unwrap();
    let current = world.sandbox_record(SID).unwrap();
    let vm = current.current_vm.unwrap();
    let public = identity::sandbox_id(NS, SID).unwrap();
    let (result, rows) = accept_original_source_row::<Evidence, _>(
        Err(rusqlite::Error::InvalidQuery), &world, NS, current, (SID, vm, &public),
        OriginalSourceRows { prepared: Some(expected.clone()), activated: Some(expected.clone()) },
        &mut JsonOriginalRowCodec,
    );
    assert!(matches!(result, Err(JournalError::Sqlite(rusqlite::Error::InvalidQuery))));
    assert_eq!(rows.prepared, Some(expected.clone()));
    assert_eq!(rows.activated, Some(expected));
}

#[test]
fn real_row_driver_stops_at_first_bad_receipt_before_later_bad_action() {
    let dir = tempfile::tempdir().unwrap();
    let expected = source();
    let mut journal = running(&dir.path().join("controller.db"), Some(&expected), Some(&expected));
    let world = journal.world().unwrap();
    // Only actual successful Prepare/Activate rows are changed. Other earlier commands
    // would deliberately skip receipt decoding in the original business handler.
    let mut statement = journal.journal.connection.prepare(
        "SELECT id,action_json FROM outbox ORDER BY id",
    ).unwrap();
    let rows = statement.query_map([], |row| Ok((row.get::<_, i64>(0)?, row.get::<_, Vec<u8>>(1)?))).unwrap();
    let mut ids = Vec::new();
    for row in rows {
        let (id, bytes) = row.unwrap();
        let action: WorkAction<core::EffectId> = serde_json::from_slice(&bytes).unwrap();
        if matches!(action.command.command(), core::Command::PrepareVm { .. } | core::Command::ActivateVm { .. }) {
            ids.push(id);
        }
    }
    drop(statement);
    assert_eq!(ids.len(), 2);
    journal.journal.connection.execute("UPDATE outbox SET receipt_json=?1 WHERE id=?2", rusqlite::params![b"{".as_slice(), ids[0]]).unwrap();
    journal.journal.connection.execute("UPDATE outbox SET action_json=?1 WHERE id=?2", rusqlite::params![b"[]".as_slice(), ids[1]]).unwrap();
    let transaction = journal.journal.connection.transaction().unwrap();
    let result = original_source::<Evidence>(&transaction, NS, &world, SID);
    let Err(JournalError::Json(error)) = result else { panic!("expected original first receipt JSON error") };
    assert!(error.is_eof(), "later invalid-action data error must not replace first receipt EOF: {error}");
}
