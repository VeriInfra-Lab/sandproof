//! Draft regression inputs for the unchanged `codec::load`; not acceptance evidence.
//! These use real `SQLite`, Serde and SHA on synthetic, explicitly configured data.
use super::*;
use crate::controller::lifecycle_inbox::{
    codec,
    model::{Inbox, LifecycleRetentionLane},
};
use crate::controller::tests_support::lifecycle_application;
use rusqlite::{Connection, params};
use sha2::{Digest, Sha256};

fn empty() -> Inbox {
    let config = config();
    let lanes = (1..=config.max_sessions)
        .map(LifecycleRetentionLane::fresh)
        .collect();
    Inbox {
        config,
        host_epoch_observation: None,
        owner_high_water: 0,
        owners: vec![],
        sessions: vec![],
        lanes,
    }
}

fn recorded() -> Inbox {
    let mut inbox = empty();
    let intent = create("owner", "session");
    assert_eq!(intent.validate(&context()), Ok(()));
    assert_eq!(inbox.record(&intent).unwrap(), (0, 0));
    inbox.validate().unwrap();
    inbox
}

fn metadata(version: i64, source: bool, inbox: bool) -> Connection {
    let connection = Connection::open_in_memory().unwrap();
    connection
        .execute_batch("CREATE TABLE controller_meta(id INTEGER PRIMARY KEY, version INTEGER);")
        .unwrap();
    connection
        .execute("INSERT INTO controller_meta VALUES(1,?1)", [version])
        .unwrap();
    if source {
        connection
            .execute_batch("CREATE TABLE controller_source_work(id INTEGER PRIMARY KEY);")
            .unwrap();
    }
    if inbox {
        connection
            .execute_batch(
                "CREATE TABLE controller_lifecycle_inbox(\
             id INTEGER PRIMARY KEY,state_json BLOB,sha256 BLOB,reserve_bytes INTEGER);",
            )
            .unwrap();
    }
    connection
}

fn write_row(connection: &Connection, bytes: &[u8], reserve: u64) {
    let digest = Sha256::digest(bytes);
    connection
        .execute(
            "INSERT OR REPLACE INTO controller_lifecycle_inbox VALUES(1,?1,?2,?3)",
            params![bytes, digest.as_slice(), i64::try_from(reserve).unwrap()],
        )
        .unwrap();
}

fn database(inbox: &Inbox) -> Connection {
    let connection = metadata(crate::LIFECYCLE_STORE_FORMAT_VERSION, true, true);
    connection
        .pragma_update(
            None,
            "user_version",
            crate::journal::MAINTAINED_FORMAT_VERSION,
        )
        .unwrap();
    connection
        .execute_batch(
            "CREATE TABLE controller_maintenance_config(id INTEGER PRIMARY KEY,config_json BLOB);",
        )
        .unwrap();
    let policy = lifecycle_application::maintenance();
    policy.validate().unwrap();
    connection
        .execute(
            "INSERT INTO controller_maintenance_config VALUES(1,?1)",
            [serde_json::to_vec(&policy).unwrap()],
        )
        .unwrap();
    write_row(
        &connection,
        &serde_json::to_vec(inbox).unwrap(),
        inbox.config.required_bytes(),
    );
    connection
}

fn rejected(connection: &Connection) {
    let changes = connection.total_changes();
    assert!(matches!(
        codec::load(connection),
        Err(JournalError::CorruptStore)
    ));
    assert_eq!(connection.total_changes(), changes);
}

#[test]
fn legacy_absence_and_format_rejections_do_not_read_later_configuration() {
    let current = crate::LIFECYCLE_STORE_FORMAT_VERSION;
    for version in [current - 1, current + 1] {
        let legacy = metadata(version, false, false);
        assert!(codec::load(&legacy).unwrap().is_none());
        rejected(&metadata(version, true, false));
        rejected(&metadata(version, false, true));
    }
    rejected(&metadata(current, false, false));
    rejected(&metadata(current, true, false));
    let internal = metadata(current, true, true);
    internal
        .execute_batch("CREATE TABLE controller_internal_inbox(id INTEGER PRIMARY KEY);")
        .unwrap();
    rejected(&internal);
}

#[test]
fn real_codec_load_returns_the_complete_empty_and_recorded_values_without_row_changes() {
    for inbox in [empty(), recorded()] {
        let connection = database(&inbox);
        let changes = connection.total_changes();
        assert_eq!(codec::load(&connection).unwrap(), Some(inbox));
        assert_eq!(connection.total_changes(), changes);
    }
}

#[test]
fn decoded_equivalence_and_matching_digest_do_not_bypass_canonical_bytes() {
    let inbox = recorded();
    let canonical = serde_json::to_vec(&inbox).unwrap();
    let mut whitespace = canonical.clone();
    whitespace.push(b' ');
    assert_eq!(serde_json::from_slice::<Inbox>(&whitespace).unwrap(), inbox);
    assert_ne!(whitespace, canonical);
    let connection = database(&inbox);
    write_row(&connection, &whitespace, inbox.config.required_bytes());
    rejected(&connection);

    let mut omitted = serde_json::to_value(&inbox).unwrap();
    assert_eq!(
        omitted
            .as_object_mut()
            .unwrap()
            .remove("host_epoch_observation"),
        Some(serde_json::Value::Null),
    );
    let omitted = serde_json::to_vec(&omitted).unwrap();
    assert_eq!(serde_json::from_slice::<Inbox>(&omitted).unwrap(), inbox);
    write_row(&connection, &omitted, inbox.config.required_bytes());
    rejected(&connection);
}

#[test]
fn all_five_required_nullable_fields_reject_absence_but_accept_explicit_null() {
    let inbox = recorded();
    let value = serde_json::to_value(&inbox).unwrap();
    for (entry, field) in [
        (true, "previous_effective_end_at_ms"),
        (true, "unload_resolution"),
        (false, "confirmed_renewal"),
        (false, "compacted_sdk_through"),
        (false, "native_retirement"),
    ] {
        let mut absent = value.clone();
        let session = &mut absent["sessions"][0];
        let object = if entry {
            session["entries"][0].as_object_mut().unwrap()
        } else {
            session.as_object_mut().unwrap()
        };
        assert_eq!(object.remove(field), Some(serde_json::Value::Null));
        let bytes = serde_json::to_vec(&absent).unwrap();
        let error = serde_json::from_slice::<Inbox>(&bytes).unwrap_err();
        assert!(
            error
                .to_string()
                .contains(&format!("missing field `{field}`"))
        );
        let connection = database(&inbox);
        write_row(&connection, &bytes, inbox.config.required_bytes());
        rejected(&connection);
    }
    let connection = database(&inbox);
    assert_eq!(codec::load(&connection).unwrap(), Some(inbox));
}

#[test]
fn invalid_syntax_unknown_fields_and_wrong_digest_are_rejected_by_actual_load() {
    let inbox = empty();
    let connection = database(&inbox);
    write_row(&connection, b"{", inbox.config.required_bytes());
    rejected(&connection);
    let mut extra = serde_json::to_value(&inbox).unwrap();
    extra["unrecognized"] = serde_json::Value::Bool(true);
    let bytes = serde_json::to_vec(&extra).unwrap();
    assert!(serde_json::from_slice::<Inbox>(&bytes).is_err());
    write_row(&connection, &bytes, inbox.config.required_bytes());
    rejected(&connection);
    write_row(
        &connection,
        &serde_json::to_vec(&inbox).unwrap(),
        inbox.config.required_bytes(),
    );
    connection
        .execute(
            "UPDATE controller_lifecycle_inbox SET sha256=?1 WHERE id=1",
            [vec![0_u8; 32]],
        )
        .unwrap();
    rejected(&connection);
}

#[test]
fn valid_canonical_encoding_and_digest_still_require_full_inbox_validation() {
    let mut inbox = recorded();
    inbox.lanes.swap(0, 1);
    assert!(matches!(inbox.validate(), Err(JournalError::CorruptStore)));
    rejected(&database(&inbox));
}

#[test]
fn reserve_must_match_the_configuration_after_signed_and_capacity_checks() {
    let inbox = empty();
    for reserve in [
        -1,
        0,
        i64::try_from(inbox.config.required_bytes() + 1).unwrap(),
        i64::MAX,
    ] {
        let connection = database(&inbox);
        connection
            .execute(
                "UPDATE controller_lifecycle_inbox SET reserve_bytes=?1 WHERE id=1",
                [reserve],
            )
            .unwrap();
        rejected(&connection);
    }
}

#[test]
fn complete_value_can_still_exceed_the_native_operation_policy() {
    let mut inbox = empty();
    inbox.config.max_operations_per_session = 9;
    inbox.validate().unwrap();
    let connection = database(&inbox);
    rejected(&connection);
}

#[test]
fn initial_configuration_failure_precedes_decoding_the_inbox() {
    let inbox = empty();
    let connection = database(&inbox);
    write_row(&connection, b"{", inbox.config.required_bytes());
    connection
        .execute(
            "UPDATE controller_maintenance_config SET config_json=?1 WHERE id=1",
            [b"{".as_slice()],
        )
        .unwrap();
    assert!(matches!(
        codec::load(&connection),
        Err(JournalError::Json(_))
    ));
}

// The same boundary assertions run against the unchanged and candidate readers.
#[test]
fn every_deployment_bound_retains_zero_and_signed_overflow_rejection() {
    type Policy = crate::controller::checkpoint::MaintenanceConfig;
    type Capacity = crate::controller::capacity::CapacityConfig;
    let policy_fields: [fn(&mut Policy, u64); 8] = [
        |p, value| p.retention_ms = value,
        |p, value| p.max_checkpoint_bytes = value,
        |p, value| p.max_records = value,
        |p, value| p.max_references = value,
        |p, value| p.max_tail_inputs = value,
        |p, value| p.max_tail_bytes = value,
        |p, value| p.max_input_bytes = value,
        |p, value| p.prune_batch_rows = value,
    ];
    let capacity_fields: [fn(&mut Capacity, u64); 12] = [
        |p, value| p.max_live_sandboxes = value,
        |p, value| p.max_sql_rows = value,
        |p, value| p.max_sql_bytes = value,
        |p, value| p.max_sql_value_bytes = value,
        |p, value| p.max_receipt_bytes = value,
        |p, value| p.max_transaction_inputs = value,
        |p, value| p.max_transaction_bytes = value,
        |p, value| p.max_database_bytes = value,
        |p, value| p.max_wal_bytes = value,
        |p, value| p.max_shm_bytes = value,
        |p, value| p.checkpoint_scratch_bytes = value,
        |p, value| p.max_online_storage_bytes = value,
    ];
    lifecycle_application::maintenance().validate().unwrap();
    for value in [0, u64::try_from(i64::MAX).unwrap() + 1, u64::MAX] {
        for set in policy_fields {
            let mut policy = lifecycle_application::maintenance();
            set(&mut policy, value);
            assert!(matches!(
                policy.validate(),
                Err(JournalError::UnsupportedStore)
            ));
        }
        for set in capacity_fields {
            let mut policy = lifecycle_application::maintenance();
            set(policy.capacity.as_mut().unwrap(), value);
            assert!(matches!(
                policy.validate(),
                Err(JournalError::UnsupportedStore)
            ));
        }
    }
}

#[test]
fn checked_arithmetic_errors_preserve_the_first_rejection_before_inbox_decode() {
    let mut product = lifecycle_application::maintenance();
    product.max_input_bytes = i64::MAX.cast_unsigned();
    product.max_tail_bytes = i64::MAX.cast_unsigned();
    product.max_tail_inputs = 4;
    product.capacity.as_mut().unwrap().max_transaction_inputs = 4;
    let mut product_early = product.clone();
    product_early
        .capacity
        .as_mut()
        .unwrap()
        .max_transaction_inputs = 3;
    let mut total = lifecycle_application::maintenance();
    let capacity = total.capacity.as_mut().unwrap();
    capacity.max_database_bytes = i64::MAX.cast_unsigned();
    capacity.max_wal_bytes = i64::MAX.cast_unsigned();
    capacity.max_shm_bytes = 2;
    let mut total_early = total.clone();
    let capacity = total_early.capacity.as_mut().unwrap();
    capacity.max_receipt_bytes = capacity.max_sql_value_bytes + 1;
    for (policy, arithmetic_error) in [
        (product, true),
        (product_early, false),
        (total, true),
        (total_early, false),
    ] {
        let direct = policy.validate();
        let connection = database(&empty());
        connection
            .execute(
                "UPDATE controller_maintenance_config SET config_json=?1 WHERE id=1",
                [serde_json::to_vec(&policy).unwrap()],
            )
            .unwrap();
        connection
            .execute(
                "UPDATE controller_lifecycle_inbox SET state_json=?1 WHERE id=1",
                [b"{".as_slice()],
            )
            .unwrap();
        let loaded = codec::load(&connection);
        if arithmetic_error {
            assert!(matches!(direct, Err(JournalError::MaintenanceCapacity)));
            assert!(matches!(loaded, Err(JournalError::MaintenanceCapacity)));
        } else {
            assert!(matches!(direct, Err(JournalError::UnsupportedStore)));
            assert!(matches!(loaded, Err(JournalError::UnsupportedStore)));
        }
    }
}

#[test]
fn equal_length_reordered_json_is_not_canonical_even_with_matching_digest() {
    let inbox = recorded();
    let canonical = serde_json::to_vec(&inbox).unwrap();
    let reordered = serde_json::to_vec(&serde_json::to_value(&inbox).unwrap()).unwrap();
    assert_eq!(reordered.len(), canonical.len());
    assert_ne!(reordered, canonical);
    assert_eq!(serde_json::from_slice::<Inbox>(&reordered).unwrap(), inbox);
    let connection = database(&inbox);
    write_row(&connection, &reordered, inbox.config.required_bytes());
    rejected(&connection);
}
