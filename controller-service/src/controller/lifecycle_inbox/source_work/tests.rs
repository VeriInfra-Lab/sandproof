use super::*;
use crate::controller::{tests_support::lifecycle_application as v3, transaction};
use crate::{NeverCrash, application::WorkEvidence, capture::HostEpochObservation};
use sandbox_controller_core as core;
use sandproof_protocol::lifecycle::{
    CaptureId, ExpectedSession, IncarnationId, LifecycleOperation,
};
use std::{
    path::Path,
    sync::{Arc, Mutex},
};
use v3::fixture::{State, TestEpoch, TestHost};
mod continue_application;
mod end_pipeline;
mod original_continue;
mod restore_exit;
mod slot_reuse;
mod terminal_drain;
type Evidence = <TestHost as WorkEvidence>::Evidence;

fn config() -> SourceObservationConfig {
    SourceObservationConfig {
        max_work_bytes: 32_768,
        max_configuration_bytes: 1024,
        max_executable_bytes: 1024 * 1024,
        max_kernel_bytes: 1024 * 1024,
        max_disk_bytes: 1024 * 1024,
        max_material_bytes: 64 * 1024 * 1024 * 1024,
    }
}
fn unload() -> AcceptedLifecycleIntent {
    let mut request = v3::request();
    request.accepted_version = 2;
    request.request.binding.expected = ExpectedSession::Version(1);
    request.request.binding.key.sequence = 2.try_into().unwrap();
    request.request.operation = LifecycleOperation::Unload {
        source_incarnation: IncarnationId::try_from(101).unwrap(),
        capture: CaptureId::try_from(1).unwrap(),
    };
    request
}
fn bootstrap(journal: &mut ControllerJournal) {
    journal
        .establish_host_epoch(
            &mut |r| {
                Ok(HostEpochObservation {
                    node: r.node().clone(),
                    registration: r.registration().clone(),
                })
            },
            &mut NeverCrash,
        )
        .unwrap();
}
fn running_source(path: &Path) -> (ControllerJournal, Arc<Mutex<State>>) {
    let state = Arc::new(Mutex::new(State {
        epoch: TestEpoch::Matching,
        source: Some(SourceProcessIdentity {
            pid: 1234,
            start_time_ticks: 5678,
            boot_id: "fixture-boot".into(),
        }),
        ..State::default()
    }));
    let app = v3::application(path, state.clone(), true);
    app.record_lifecycle_intent(&v3::request(), &v3::context())
        .unwrap();
    app.establish_lifecycle_host_epoch().unwrap();
    v3::advance(&app).unwrap();
    drop(app);
    let mut journal = v3::open(path);
    bootstrap(&mut journal);
    (journal, state)
}
fn ready(path: &Path) -> (ControllerJournal, Arc<Mutex<State>>) {
    let (mut journal, state) = running_source(path);
    journal
        .record_lifecycle_intent(&unload(), &v3::context(), &mut NeverCrash)
        .unwrap();
    (journal, state)
}
fn prepare(journal: &mut ControllerJournal) -> SourceWorkRecord {
    journal
        .prepare_lifecycle_source_work::<Evidence>(&unload(), &v3::context(), 10, &mut NeverCrash)
        .unwrap()
}
fn observation(requirement: &SourceObservationRequirement) -> RawSourceObservation {
    RawSourceObservation {
        work_binding: requirement.work_binding,
        resource: requirement.resource.clone(),
        source: requirement.source.clone(),
        configuration: br#"{"vcpu_count":1}"#.to_vec(),
        executable_sha256: [11; 32],
        kernel_sha256: [12; 32],
        kernel_bytes: 100,
        disk_bytes: 4096,
    }
}

#[test]
fn source_work_keeps_its_exact_create_and_unload_rows_while_old_sdk_checkpoint_is_reclaimed() {
    use sandproof_protocol::lifecycle::RenewalConfirmation;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("db");
    let (mut journal, _) = running_source(&path);
    let create = v3::request();
    let mut sdk = create.clone();
    sdk.accepted_version = 2;
    sdk.request.binding.expected = ExpectedSession::Version(1);
    sdk.request.binding.key.window = 2.try_into().unwrap();
    sdk.request.binding.key.sequence = 2.try_into().unwrap();
    sdk.request.operation = LifecycleOperation::Renew {
        incarnation: 101.try_into().unwrap(),
        end_at_ms: 90_010,
        extend_only: false,
    };
    journal
        .record_lifecycle_intent(&sdk, &v3::context(), &mut NeverCrash)
        .unwrap();
    let first = RenewalConfirmation {
        effective_end_at_ms: journal
            .apply_lifecycle_renew(&sdk, &v3::context(), 100, &mut NeverCrash)
            .unwrap()
            .1,
        request: sdk.clone(),
    };
    journal
        .confirm_lifecycle_renewal(&first, &v3::context(), &mut NeverCrash)
        .unwrap();
    let mut next = sdk.clone();
    next.accepted_version = 3;
    next.request.binding.expected = ExpectedSession::Version(2);
    next.request.binding.key.sequence = 3.try_into().unwrap();
    next.request.operation = LifecycleOperation::Renew {
        incarnation: 101.try_into().unwrap(),
        end_at_ms: 100_010,
        extend_only: false,
    };
    journal
        .record_lifecycle_intent(&next, &v3::context(), &mut NeverCrash)
        .unwrap();
    let result = RenewalConfirmation {
        effective_end_at_ms: journal
            .apply_lifecycle_renew(&next, &v3::context(), 100, &mut NeverCrash)
            .unwrap()
            .1,
        request: next,
    };
    let mut unload = unload();
    unload.accepted_version = 4;
    unload.request.binding.expected = ExpectedSession::Version(3);
    unload.request.binding.key.sequence = 4.try_into().unwrap();
    journal
        .record_lifecycle_intent(&unload, &v3::context(), &mut NeverCrash)
        .unwrap();
    let work = journal
        .prepare_lifecycle_source_work::<Evidence>(&unload, &v3::context(), 100, &mut NeverCrash)
        .unwrap();
    assert!(retains_session(&journal.journal.connection, &create.request.binding.session).unwrap());
    assert!(references_intent(&journal.journal.connection, &create).unwrap());
    assert!(references_intent(&journal.journal.connection, &unload).unwrap());
    assert!(!references_intent(&journal.journal.connection, &sdk).unwrap());
    journal
        .confirm_lifecycle_renewal(&result, &v3::context(), &mut NeverCrash)
        .unwrap();
    assert!(
        matches!(
            journal.lifecycle_work_state(&sdk, &v3::context()),
            Err(JournalError::Conflict)
        ),
        "an unrelated source retirement pin must not permanently retain the old SDK row"
    );
    assert_eq!(
        journal
            .lifecycle_work_state(&unload, &v3::context())
            .unwrap()
            .intent,
        unload
    );
    assert!(
        journal
            .lifecycle_work_state(&create, &v3::context())
            .is_ok()
    );
    assert_eq!(
        journal.lifecycle_source_work(&v3::config().node).unwrap(),
        vec![work.clone()]
    );
    codec::validate(&journal.journal.connection, &journal.world().unwrap()).unwrap();
    drop(journal);
    let journal = v3::open(&path);
    assert_eq!(
        journal.lifecycle_source_work(&v3::config().node).unwrap(),
        vec![work]
    );
}

#[test]
fn source_and_inbox_initialize_atomically_with_exact_immutable_limits() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("db");
    let mut journal = v3::open(&path);
    let before = journal.world().unwrap();
    journal
        .initialize_lifecycle_inbox(v3::config(), config(), &mut NeverCrash)
        .unwrap();
    assert_eq!(journal.world().unwrap(), before);
    assert!(
        journal
            .lifecycle_source_work(&v3::config().node)
            .unwrap()
            .is_empty()
    );
    journal
        .check_lifecycle_source_observations(&config())
        .unwrap();
    let mut changed = config();
    changed.max_configuration_bytes += 1;
    assert!(matches!(
        journal.check_lifecycle_source_observations(&changed),
        Err(JournalError::UnsupportedStore)
    ));
    assert!(matches!(
        journal.initialize_lifecycle_inbox(v3::config(), config(), &mut NeverCrash),
        Err(JournalError::UnsupportedStore)
    ));
    let reserve: i64 = journal
        .journal
        .connection
        .query_row(
            "SELECT 8192+sum(reserve_bytes) FROM controller_source_work_records",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        u64::try_from(reserve).unwrap(),
        8192 + 2 * (8 - 2) * config().required_bytes(1)
    );
    drop(journal);
    v3::open(&path)
        .check_lifecycle_source_observations(&config())
        .unwrap();
}

#[test]
fn inbox_history_cannot_exceed_the_native_deployment_bound() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("db");
    let mut journal = v3::open(&path);
    let before = journal.world().unwrap();
    let mut inbox = v3::config();
    inbox.max_operations_per_session = 9;
    assert!(matches!(
        journal.initialize_lifecycle_inbox(inbox, config(), &mut NeverCrash),
        Err(JournalError::UnsupportedStore)
    ));
    assert_eq!(journal.world().unwrap(), before);
    assert!(journal.lifecycle_inbox_configuration().unwrap().is_none());
    journal
        .initialize_lifecycle_inbox(v3::config(), config(), &mut NeverCrash)
        .unwrap();
    drop(journal);
    let journal = v3::open(&path);
    assert_eq!(
        journal.lifecycle_inbox_configuration().unwrap().unwrap(),
        v3::config()
    );
}

#[test]
fn per_work_reservation_cannot_shrink_below_its_declared_envelope() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("db");
    let mut journal = v3::open(&path);
    journal
        .initialize_lifecycle_inbox(v3::config(), config(), &mut NeverCrash)
        .unwrap();
    // Leave the original canonical source bytes and checksum unchanged. A
    // smaller reserve must not silently authorize its original work row.
    journal
        .journal
        .connection
        .execute(
            "UPDATE controller_source_work_records SET reserve_bytes=?1 WHERE id=1",
            [i64::try_from(config().required_bytes(1) - 1).unwrap()],
        )
        .unwrap();
    assert!(matches!(
        codec::load(&journal.journal.connection),
        Err(JournalError::CorruptStore)
    ));
    drop(journal);
    let host = v3::fixture::complete_fixture::host(
        &path,
        "0123456789abcdef",
        64,
        2 * 1024 * 1024,
        v3::CLEANUP_BYTES + 1000,
    );
    assert!(matches!(
        ControllerJournal::open_complete(&path, "0123456789abcdef", host, v3::maintenance(), 0),
        Err(JournalError::CorruptStore)
    ));
}

#[test]
fn source_and_inbox_unknown_commit_recovers_both_or_neither_without_fabricated_work() {
    for durable in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("db");
        let mut journal = v3::open(&path);
        transaction::fail_next_commit(durable);
        assert!(matches!(
            journal.initialize_lifecycle_inbox(v3::config(), config(), &mut NeverCrash),
            Err(JournalError::CommitUncertain(_))
        ));
        drop(journal);
        let mut journal = v3::open(&path);
        for table in [
            "controller_lifecycle_inbox",
            "controller_source_work",
            "controller_source_work_records",
        ] {
            let exists: bool = journal
                .journal
                .connection
                .query_row(
                    "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE name=?1)",
                    [table],
                    |r| r.get(0),
                )
                .unwrap();
            assert_eq!(exists, durable, "{table}");
        }
        if !durable {
            journal
                .initialize_lifecycle_inbox(v3::config(), config(), &mut NeverCrash)
                .unwrap();
        }
        journal
            .check_lifecycle_source_observations(&config())
            .unwrap();
        assert!(
            journal
                .lifecycle_source_work(&v3::config().node)
                .unwrap()
                .is_empty()
        );
    }
}

#[test]
fn accepted_source_work_replay_and_restart_keep_one_question_and_raw_result() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("db");
    let (mut journal, state) = ready(&path);
    let record = prepare(&mut journal);
    assert_eq!(
        record.work.anchor.native.current_vm,
        Some(sandbox_controller_core::VmId(0)),
        "the original scheduler allocates the first VM at zero"
    );
    assert!(record.observation.is_none());
    assert_eq!(prepare(&mut journal), record);
    let saved = journal
        .save_source_observation::<Evidence>(
            &record.work,
            &v3::context(),
            observation(record.work.requirement()),
            10,
            &mut NeverCrash,
        )
        .unwrap();
    assert_eq!(prepare(&mut journal), saved);
    assert_eq!(
        journal.lifecycle_source_work(&v3::config().node).unwrap(),
        vec![saved.clone()]
    );
    let actions = state.lock().unwrap().actions.len();
    drop(journal);
    let journal = v3::open(&path);
    assert_eq!(
        journal.lifecycle_source_work(&v3::config().node).unwrap(),
        vec![saved]
    );
    assert_eq!(state.lock().unwrap().actions.len(), actions);
    assert_eq!(
        journal
            .world()
            .unwrap()
            .sandbox_record(sandbox_controller_core::SandboxId(1))
            .unwrap()
            .phase,
        sandbox_controller_core::Phase::Running
    );
}

#[test]
fn source_late_destroy_lifetime_change_and_expiry_cannot_accept_old_facts() {
    for mode in 0..3 {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("db");
        let (mut journal, state) = ready(&path);
        let record = prepare(&mut journal);
        let deadline = record.work.anchor.native.lifetime.unwrap().deadline_ms;
        let now = match mode {
            0 => {
                let mut destroy = unload();
                destroy.accepted_version = 3;
                destroy.request.binding.expected = ExpectedSession::Version(2);
                destroy.request.binding.key.sequence = 3.try_into().unwrap();
                destroy.request.operation = LifecycleOperation::Destroy {};
                journal
                    .record_lifecycle_intent(&destroy, &v3::context(), &mut NeverCrash)
                    .unwrap();
                10
            }
            1 => {
                let local = record.work.anchor.local;
                journal
                    .connect_lifetime(
                        &sandbox_controller_core::Actor::Tenant(local.owner),
                        local.sandbox,
                        20,
                        Some(120),
                        v3::limits(),
                    )
                    .unwrap();
                20
            }
            _ => deadline,
        };
        let actions = state.lock().unwrap().actions.len();
        assert!(
            journal
                .save_source_observation::<Evidence>(
                    &record.work,
                    &v3::context(),
                    observation(record.work.requirement()),
                    now,
                    &mut NeverCrash
                )
                .is_err()
        );
        assert!(
            journal.lifecycle_source_work(&v3::config().node).unwrap()[0]
                .observation
                .is_none()
        );
        assert_eq!(state.lock().unwrap().actions.len(), actions);
    }
}

#[test]
fn mismatched_or_oversized_raw_observations_never_replace_original_question() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("db");
    let (mut journal, _) = ready(&path);
    let record = prepare(&mut journal);
    for mode in 0..6 {
        let mut raw = observation(record.work.requirement());
        match mode {
            0 => raw.source.pid += 1,
            1 => raw.work_binding = ContentDigest::from_sha256([0; 32]),
            2 => raw.configuration.clear(),
            3 => raw.configuration = vec![0; 1025],
            4 => raw.kernel_bytes = config().max_kernel_bytes + 1,
            _ => raw.disk_bytes = config().max_disk_bytes + 1,
        }
        assert!(matches!(
            journal.save_source_observation::<Evidence>(
                &record.work,
                &v3::context(),
                raw,
                10,
                &mut NeverCrash
            ),
            Err(JournalError::InvalidHostEvidence)
        ));
    }
    assert!(
        journal.lifecycle_source_work(&v3::config().node).unwrap()[0]
            .observation
            .is_none()
    );
}

#[test]
fn source_provider_runs_after_commit_without_sql_and_unknown_keeps_original_work() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("db");
    let (mut journal, state) = ready(&path);
    let record = prepare(&mut journal);
    drop(journal);
    let app = v3::application(&path, state.clone(), false);
    app.establish_lifecycle_host_epoch().unwrap();
    assert!(matches!(
        app.observe_lifecycle_source_work(&record.work, &v3::context()),
        Err(JournalError::UnresolvedAction)
    ));
    let target = path.clone();
    let expected = record.clone();
    state.lock().unwrap().source_probe = Some(Box::new(move |r| {
        let mut connection = rusqlite::Connection::open(target).unwrap();
        connection.busy_timeout(std::time::Duration::ZERO).unwrap();
        connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .unwrap()
            .rollback()
            .unwrap();
        let saved = codec::load(&connection).unwrap().unwrap();
        assert_eq!(saved.records, vec![expected]);
        Ok(observation(r))
    }));
    assert!(
        app.observe_lifecycle_source_work(&record.work, &v3::context())
            .unwrap()
            .observation
            .is_some()
    );
    assert_eq!(state.lock().unwrap().source_calls, 2);
    assert!(
        app.observe_lifecycle_source_work(&record.work, &v3::context())
            .unwrap()
            .observation
            .is_some()
    );
    assert_eq!(state.lock().unwrap().source_calls, 2);
}

#[test]
fn source_work_unknown_commits_keep_actual_original_question_and_result() {
    for durable in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("db");
        let (mut journal, _) = ready(&path);
        transaction::fail_next_commit(durable);
        assert!(matches!(
            journal.prepare_lifecycle_source_work::<Evidence>(
                &unload(),
                &v3::context(),
                10,
                &mut NeverCrash
            ),
            Err(JournalError::CommitUncertain(_))
        ));
        drop(journal);
        let mut journal = v3::open(&path);
        bootstrap(&mut journal);
        assert_eq!(
            journal
                .lifecycle_source_work(&v3::config().node)
                .unwrap()
                .len(),
            usize::from(durable)
        );
        let record = prepare(&mut journal);
        transaction::fail_next_commit(durable);
        assert!(matches!(
            journal.save_source_observation::<Evidence>(
                &record.work,
                &v3::context(),
                observation(record.work.requirement()),
                10,
                &mut NeverCrash
            ),
            Err(JournalError::CommitUncertain(_))
        ));
        drop(journal);
        let mut journal = v3::open(&path);
        bootstrap(&mut journal);
        let saved = journal.lifecycle_source_work(&v3::config().node).unwrap();
        assert_eq!(saved.len(), 1);
        assert_eq!(saved[0].work, record.work);
        assert_eq!(saved[0].observation.is_some(), durable);
        assert!(
            journal
                .save_source_observation::<Evidence>(
                    &record.work,
                    &v3::context(),
                    observation(record.work.requirement()),
                    10,
                    &mut NeverCrash
                )
                .unwrap()
                .observation
                .is_some()
        );
    }
}

#[test]
fn source_18_missing_shapes_hash_budget_node_vm_and_17_stray_table_fail_closed() {
    for mode in 0..8 {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("db");
        let (mut journal, _) = ready(&path);
        let record = prepare(&mut journal);
        journal
            .save_source_observation::<Evidence>(
                &record.work,
                &v3::context(),
                observation(record.work.requirement()),
                10,
                &mut NeverCrash,
            )
            .unwrap();
        drop(journal);
        let db = rusqlite::Connection::open(&path).unwrap();
        match mode {
            0=>db.execute_batch("DROP TABLE controller_source_work").unwrap(),
            1=>db.execute_batch("UPDATE controller_meta SET version=17").unwrap(),
            2=>db.execute_batch("UPDATE controller_source_work SET state_json=x'5b5d'").unwrap(),
            3=>db.execute_batch("UPDATE controller_source_work SET reserve_bytes=reserve_bytes-1").unwrap(),
            4=>db.execute_batch("ALTER TABLE controller_source_work RENAME COLUMN reserve_bytes TO unrecognized").unwrap(),
            _=>{
                if mode == 5 {
                    let bytes:Vec<u8> = db.query_row("SELECT state_json FROM controller_source_work WHERE id=1",[],|r|r.get(0)).unwrap();
                    let mut value:serde_json::Value=serde_json::from_slice(&bytes).unwrap();
                    value["node"]["node_generation"]=2.into();
                    let bytes=serde_json::to_vec(&value).unwrap();
                    db.execute("UPDATE controller_source_work SET state_json=?1,sha256=?2",rusqlite::params![bytes,Sha256::digest(&bytes).as_slice()]).unwrap();
                } else {
                    let mut saved=codec::load(&db).unwrap().unwrap();
                    if mode == 6 {
                        saved.records[0].work.anchor.native.current_vm=Some(sandbox_controller_core::VmId(12345));
                        saved.records[0].work.requirement.work_binding=digest(&serde_json::to_vec(&saved.records[0].work.anchor).unwrap());
                    } else { saved.records[0].observation.as_mut().unwrap().configuration=vec![0;1025]; }
                    let record=&saved.records[0];
                    let bytes=serde_json::to_vec(record).unwrap();
                    db.execute("UPDATE controller_source_work_records SET state_json=?1,sha256=?2,work_key=?3 WHERE id=1",rusqlite::params![bytes,Sha256::digest(&bytes).as_slice(),record.work.requirement.work_binding.as_bytes().as_slice()]).unwrap();
                }
            }
        }
        drop(db);
        let reserve = v3::CLEANUP_BYTES + 1000;
        let maintenance = v3::maintenance();
        assert!(
            ControllerJournal::open_complete(
                &path,
                "0123456789abcdef",
                v3::fixture::complete_fixture::host(
                    &path,
                    "0123456789abcdef",
                    64,
                    2 * 1024 * 1024,
                    reserve
                ),
                maintenance,
                0
            )
            .is_err(),
            "mode {mode}"
        );
    }
}

#[test]
fn old_or_modified_owner_vm_generation_or_full_key_cannot_store_facts() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("db");
    let (mut journal, _) = ready(&path);
    let original = prepare(&mut journal);
    for mode in 0..5 {
        let mut work = original.work.clone();
        match mode {
            0 => work.anchor.local.owner = sandbox_controller_core::TenantId(123),
            1 => work.anchor.native.current_vm = Some(sandbox_controller_core::VmId(123)),
            2 => work.anchor.native.generation += 1,
            3 => {
                work.anchor.request.request.binding.session = "foreign-session".try_into().unwrap();
            }
            _ => work.anchor.request.request.binding.key.sequence = 99.try_into().unwrap(),
        }
        assert!(
            journal
                .save_source_observation::<Evidence>(
                    &work,
                    &v3::context(),
                    observation(work.requirement()),
                    10,
                    &mut NeverCrash
                )
                .is_err()
        );
    }
    assert_eq!(
        journal.lifecycle_source_work(&v3::config().node).unwrap(),
        vec![original]
    );
}

#[test]
fn current_source_store_rejects_old_or_unknown_tags_without_changing_bytes() {
    for version in [0, 17, 19, 21, 22] {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("db");
        let (journal, _) = running_source(&path);
        drop(journal);
        let conn = rusqlite::Connection::open(&path).unwrap();
        conn.execute("UPDATE controller_meta SET version=?1", [version])
            .unwrap();
        drop(conn);
        let reserve = v3::CLEANUP_BYTES + 1000;
        let host = v3::fixture::complete_fixture::host(
            &path,
            "0123456789abcdef",
            64,
            2 * 1024 * 1024,
            reserve,
        );
        assert!(
            ControllerJournal::preflight_complete(
                &path,
                "0123456789abcdef",
                &host,
                &v3::maintenance()
            )
            .is_err()
        );
        assert!(
            ControllerJournal::open_complete(&path, "0123456789abcdef", host, v3::maintenance(), 0)
                .is_err()
        );
        let conn = rusqlite::Connection::open(&path).unwrap();
        assert_eq!(
            conn.query_row("SELECT version FROM controller_meta", [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            version
        );
    }
}

#[test]
fn source_capacity_rejection_leaves_no_partial_inbox_or_source_table() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("db");
    let mut journal = v3::open(&path);
    let before = journal.world().unwrap();
    let mut excessive = config();
    excessive.max_configuration_bytes = 1_048_576;
    assert!(matches!(
        journal.initialize_lifecycle_inbox(v3::config(), excessive, &mut NeverCrash),
        Err(JournalError::MaintenanceCapacity)
    ));
    assert_eq!(journal.world().unwrap(), before);
    for table in [
        "controller_lifecycle_inbox",
        "controller_source_work",
        "controller_source_work_records",
    ] {
        assert!(
            !journal
                .journal
                .connection
                .query_row(
                    "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE name=?1)",
                    [table],
                    |r| r.get::<_, bool>(0)
                )
                .unwrap()
        );
    }
}

fn observed_for_full(journal: &mut ControllerJournal) -> SourceWorkRecord {
    let record = prepare(journal);
    journal
        .save_source_observation::<Evidence>(
            &record.work,
            &v3::context(),
            observation(record.work.requirement()),
            10,
            &mut NeverCrash,
        )
        .unwrap()
}
fn full_request(record: &SourceWorkRecord) -> crate::capture::FullCaptureWorkRequest {
    use sandproof_protocol::lifecycle::{ContentDigest, VmCompatibility, WritableDrive};
    crate::capture::FullCaptureWorkRequest {
        pause: record.work.pause_request().unwrap(),
        compatibility: VmCompatibility {
            snapshot_format: 1,
            tool: ContentDigest::from_sha256([1; 32]),
            cpu: ContentDigest::from_sha256([2; 32]),
            devices: ContentDigest::from_sha256([3; 32]),
            image_configuration: ContentDigest::from_sha256([4; 32]),
            writable_drives: vec![WritableDrive {
                drive_id: "rootfs".try_into().unwrap(),
                jail_relative_path: "rootfs.ext4".into(),
            }],
        },
        state_bytes: 4096,
        memory_bytes: 4096,
        disk_bytes: 4096,
        api_timeout_ms: 1000,
        worker_timeout_ms: 1000,
    }
}
#[test]
fn full_budget_refusal_precedes_pause_and_does_not_leave_half_reservation() {
    let dir = tempfile::tempdir().unwrap();
    let (mut journal, _) = ready(&dir.path().join("db"));
    let record = observed_for_full(&mut journal);
    let before = journal.world().unwrap();
    let mut full = full_request(&record);
    full.state_bytes = config().max_material_bytes;
    assert!(matches!(
        journal.admit_lifecycle_full_capture::<Evidence>(
            &record.work,
            &v3::context(),
            &full,
            10,
            &mut NeverCrash
        ),
        Err(JournalError::MaintenanceCapacity)
    ));
    assert_eq!(journal.world().unwrap(), before);
    assert!(
        journal
            .lifecycle_capture_record(record.work.anchor.local.sandbox)
            .unwrap()
            .unwrap()
            .full_capture()
            .is_none()
    );
}
#[test]
fn full_original_request_replay_keeps_one_pause_and_one_material_reservation() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("db");
    let (mut journal, _) = ready(&path);
    let record = observed_for_full(&mut journal);
    let full = full_request(&record);
    let accepted = journal
        .admit_lifecycle_full_capture::<Evidence>(
            &record.work,
            &v3::context(),
            &full,
            10,
            &mut NeverCrash,
        )
        .unwrap();
    let before = journal.world().unwrap();
    assert_eq!(
        journal
            .admit_lifecycle_full_capture::<Evidence>(
                &record.work,
                &v3::context(),
                &full,
                10,
                &mut NeverCrash
            )
            .unwrap(),
        accepted
    );
    assert_eq!(journal.world().unwrap(), before);
    drop(journal);
    let journal = v3::open(&path);
    assert_eq!(
        journal
            .lifecycle_capture_record(record.work.anchor.local.sandbox)
            .unwrap(),
        Some(accepted)
    );
    assert_eq!(journal.world().unwrap(), before);
}
#[test]
fn full_rejects_changed_source_before_any_pause() {
    let dir = tempfile::tempdir().unwrap();
    let (mut journal, _) = ready(&dir.path().join("db"));
    let record = observed_for_full(&mut journal);
    let before = journal.world().unwrap();
    let mut full = full_request(&record);
    full.pause.source.start_time_ticks += 1;
    assert!(matches!(
        journal.admit_lifecycle_full_capture::<Evidence>(
            &record.work,
            &v3::context(),
            &full,
            10,
            &mut NeverCrash
        ),
        Err(JournalError::Conflict)
    ));
    assert_eq!(journal.world().unwrap(), before);
}
#[test]
fn full_pause_commit_reply_loss_preserves_exact_durable_or_uncommitted_outcome() {
    for durable in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("db");
        let (mut journal, _) = ready(&path);
        let record = observed_for_full(&mut journal);
        let full = full_request(&record);
        transaction::fail_next_commit(durable);
        assert!(matches!(
            journal.admit_lifecycle_full_capture::<Evidence>(
                &record.work,
                &v3::context(),
                &full,
                10,
                &mut NeverCrash
            ),
            Err(JournalError::CommitUncertain(_))
        ));
        drop(journal);
        let journal = v3::open(&path);
        let captured = journal
            .lifecycle_capture_record(record.work.anchor.local.sandbox)
            .unwrap()
            .unwrap();
        assert_eq!(captured.full_capture().is_some(), durable);
        assert_eq!(
            journal
                .world()
                .unwrap()
                .sandbox_record(record.work.anchor.local.sandbox)
                .unwrap()
                .phase,
            if durable {
                sandbox_controller_core::Phase::Pausing
            } else {
                sandbox_controller_core::Phase::Running
            }
        );
    }
}

// Synthetic physical observations exercise the original reducer/SQL protocol;
// this component test neither runs a VMM nor claims a physical Full passed.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct FullEvidence {
    pub(super) resource: sandproof_protocol::ResourceRef,
    pub(super) command: sandbox_controller_core::Command,
}
impl sandproof_protocol::ResourceEvidence for FullEvidence {
    fn resource(&self) -> &sandproof_protocol::ResourceRef {
        &self.resource
    }
}
impl HostEvidence for FullEvidence {
    fn validates(&self, command: &sandbox_controller_core::Command) -> bool {
        self.command == *command
    }
    fn source_process_identity(
        &self,
        command: &sandbox_controller_core::Command,
        expected_resource: &sandproof_protocol::ResourceRef,
    ) -> Option<SourceProcessIdentity> {
        if self.command != *command || &self.resource != expected_resource {
            return None;
        }
        let (sandbox_controller_core::Command::PrepareVm { vm, .. }
        | sandbox_controller_core::Command::ActivateVm { vm, .. }) = command
        else {
            return None;
        };
        Some(SourceProcessIdentity {
            pid: u32::try_from(vm.0).ok()?.checked_add(4000)?,
            start_time_ticks: vm.0.checked_add(9000)?,
            boot_id: "fixture-boot".into(),
        })
    }
    fn capture_manifest_digest(
        &self,
        command: &sandbox_controller_core::Command,
    ) -> Option<ContentDigest> {
        (self.command == *command
            && matches!(
                command,
                sandbox_controller_core::Command::SealSnapshot { .. }
                    | sandbox_controller_core::Command::CommitSnapshot { .. }
            ))
        .then_some(ContentDigest::from_sha256([88; 32]))
    }
    fn capture_material_bytes(&self, command: &sandbox_controller_core::Command) -> Option<u64> {
        self.capture_manifest_digest(command).map(|_| 12288)
    }
}
fn finish_full_component(journal: &mut ControllerJournal, record: &SourceWorkRecord) {
    use sandbox_controller_core::workflow::{self, WorkflowStep};
    let sandbox = record.work.anchor.local.sandbox;
    for _ in 0..32 {
        match workflow::next_step(&journal.world().unwrap(), sandbox) {
            WorkflowStep::Apply(input) => {
                journal.apply(*input, &mut NeverCrash).unwrap();
            }
            WorkflowStep::Claim => {
                let action = journal
                    .claim_next(Some(sandbox), &mut NeverCrash)
                    .unwrap()
                    .unwrap();
                if matches!(
                    action.command.command(),
                    sandbox_controller_core::Command::SealSnapshot { .. }
                ) {
                    let worker = crate::capture::CaptureWorkerBinding::new(
                        SourceProcessIdentity {
                            pid: 2345,
                            start_time_ticks: 9876,
                            boot_id: "fixture-boot".into(),
                        },
                        ContentDigest::from_sha256([9; 32]),
                    )
                    .unwrap();
                    let permit = journal
                        .authorize_lifecycle_full_capture(
                            &record.work,
                            &v3::context(),
                            &action,
                            worker.clone(),
                            10,
                            &mut NeverCrash,
                        )
                        .unwrap();
                    assert_eq!(permit.worker(), &worker);
                    assert!(matches!(
                        journal.authorize_lifecycle_full_capture(
                            &record.work,
                            &v3::context(),
                            &action,
                            worker,
                            10,
                            &mut NeverCrash
                        ),
                        Err(JournalError::Conflict)
                    ));
                }
                journal
                    .complete(
                        &sandproof_protocol::WorkReceipt {
                            identity: action.identity.clone(),
                            outcome: sandproof_protocol::WorkOutcome::ObservedSuccess(
                                FullEvidence {
                                    resource: action.identity.resource.clone(),
                                    command: action.command.command().clone(),
                                },
                            ),
                        },
                        &mut NeverCrash,
                    )
                    .unwrap();
                if matches!(
                    action.command.command(),
                    sandbox_controller_core::Command::SealSnapshot { .. }
                ) {
                    let ended = journal.lifecycle_capture_record(sandbox).unwrap().unwrap();
                    journal
                        .record_lifecycle_capture_request_end(
                            &record.work,
                            &v3::context(),
                            actual_api_end(&ended),
                            &mut NeverCrash,
                        )
                        .unwrap();
                }
            }
            WorkflowStep::Complete => return,
            other => panic!("original full workflow blocked: {other:?}"),
        }
    }
    panic!("original bounded full workflow did not finish");
}
#[test]
fn full_unloaded_survives_slot_reuse_before_first_center_result_and_manager_restart() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("db");
    let (mut journal, state) = ready(&path);
    let source = observed_for_full(&mut journal);
    let full = full_request(&source);
    let accepted = journal
        .admit_lifecycle_full_capture::<Evidence>(
            &source.work,
            &v3::context(),
            &full,
            10,
            &mut NeverCrash,
        )
        .unwrap();
    finish_full_component(&mut journal, &accepted);
    let local = source.work.anchor.local;
    let released = journal.host_slot_context(local.sandbox).unwrap().unwrap();
    assert!(
        journal
            .lifecycle_unloaded(&unload(), &v3::context(), 10)
            .unwrap()
            .is_none()
    );
    journal
        .release_host_slot(released, |_| Ok(()), &mut NeverCrash)
        .unwrap();
    let usage = journal.retained_resource_usage().unwrap();
    assert_eq!((usage.instances, usage.cpu, usage.memory_bytes), (0, 0, 0));
    assert_eq!(usage.disk_bytes, 4096);
    let original = journal
        .lifecycle_unloaded(&unload(), &v3::context(), 10)
        .unwrap()
        .unwrap();
    drop(journal);
    let host = v3::fixture::complete_fixture::host(
        &path,
        "0123456789abcdef",
        64,
        2 * 1024 * 1024,
        v3::CLEANUP_BYTES + 1000,
    );
    let app = v3::application_with_active_limit(&path, state, false, v3::config(), host, 1);
    app.establish_lifecycle_host_epoch().unwrap();
    let mut b = v3::request();
    b.request.binding.session = "session-b".try_into().unwrap();
    b.request.binding.key.sequence = 3.try_into().unwrap();
    b.retention.slot = 2;
    app.record_lifecycle_intent(&b, &v3::context()).unwrap();
    app.advance_lifecycle_create_with_verification(
        &b,
        &v3::context(),
        Some(v3::fixture::template()),
        crate::templates::ComputeLimits {
            max_vcpus_global: 1,
            max_vcpus_per_tenant: 1,
            max_memory_bytes_global: 129 * 1024 * 1024,
            max_memory_bytes_per_tenant: 129 * 1024 * 1024,
        },
        None,
    )
    .unwrap();
    drop(app);
    let journal = v3::open(&path);
    assert!(
        journal
            .host_slot_inventory()
            .unwrap()
            .iter()
            .any(|(context, phase)| context.position == released.position
                && context.binding.vm != released.binding.vm
                && *phase == sandbox_controller_core::maintenance::slots::HostSlotPhase::Occupied)
    );
    assert_eq!(
        journal
            .lifecycle_unloaded(&unload(), &v3::context(), 10)
            .unwrap(),
        Some(original.clone())
    );
    drop(journal);
    let journal = v3::open(&path);
    assert_eq!(
        journal
            .lifecycle_unloaded(&unload(), &v3::context(), 10)
            .unwrap(),
        Some(original)
    );
}

fn claim_capture_before_receipt(
    journal: &mut ControllerJournal,
    record: &SourceWorkRecord,
    stage: usize,
) -> sandproof_protocol::WorkAction<sandbox_controller_core::EffectId> {
    use sandbox_controller_core::{
        Command,
        workflow::{self, WorkflowStep},
    };
    let sandbox = record.work.anchor.local.sandbox;
    for _ in 0..32 {
        match workflow::next_step(&journal.world().unwrap(), sandbox) {
            WorkflowStep::Apply(input) => {
                journal.apply(*input, &mut NeverCrash).unwrap();
            }
            WorkflowStep::Claim => {
                let action = journal
                    .claim_next(Some(sandbox), &mut NeverCrash)
                    .unwrap()
                    .unwrap();
                if matches!(action.command.command(), Command::SealSnapshot { .. }) {
                    let worker = crate::capture::CaptureWorkerBinding::new(
                        SourceProcessIdentity {
                            pid: 2345,
                            start_time_ticks: 9876,
                            boot_id: "fixture-boot".into(),
                        },
                        ContentDigest::from_sha256([9; 32]),
                    )
                    .unwrap();
                    journal
                        .authorize_lifecycle_full_capture(
                            &record.work,
                            &v3::context(),
                            &action,
                            worker,
                            10,
                            &mut NeverCrash,
                        )
                        .unwrap();
                }
                let wanted = match stage {
                    0 => matches!(action.command.command(), Command::FreezeVm { .. }),
                    1 => matches!(action.command.command(), Command::SealSnapshot { .. }),
                    2 => matches!(action.command.command(), Command::CommitSnapshot { .. }),
                    _ => panic!("unknown finite capture stage"),
                };
                if wanted {
                    return action;
                }
                journal
                    .complete(
                        &sandproof_protocol::WorkReceipt {
                            identity: action.identity.clone(),
                            outcome: sandproof_protocol::WorkOutcome::ObservedSuccess(
                                FullEvidence {
                                    resource: action.identity.resource.clone(),
                                    command: action.command.command().clone(),
                                },
                            ),
                        },
                        &mut NeverCrash,
                    )
                    .unwrap();
            }
            other => panic!("original workflow blocked before selected late receipt: {other:?}"),
        }
    }
    panic!("original bounded workflow did not reach selected capture stage");
}

fn assert_late_capture_retained(
    journal: &ControllerJournal,
    accepted: &SourceWorkRecord,
    destroy: &AcceptedLifecycleIntent,
    terminal_row: &sandbox_controller_core::SandboxRecord,
    budget: u64,
    stage: usize,
) {
    use sandbox_controller_core::{
        Command, OperationKey,
        capture::{CaptureCommit, RestoreEligibility},
    };
    let sandbox = accepted.work.anchor.local.sandbox;
    let world = journal.world().unwrap();
    let row = world.sandbox_record(sandbox).unwrap();
    assert_eq!(
        row, terminal_row,
        "late receipt must not change current terminal authority"
    );
    assert!(row.terminal);
    assert_eq!(row.phase, sandbox_controller_core::Phase::Revoking);
    let key = OperationKey {
        sandbox,
        operation: accepted.capture_operation().unwrap(),
    };
    let plan = world.operation_plan(key).unwrap();
    let capture = plan.capture.unwrap();
    assert_eq!(capture.eligibility(), RestoreEligibility::Revoked);
    assert!(!capture.may_stop_original());
    assert!(!capture.may_reactivate_original());
    assert!(
        capture.has_capture_work(),
        "material responsibility pin remains held"
    );
    assert_eq!(capture.has_unload_source(), stage == 2);
    if stage == 2 {
        assert_eq!(capture.commit(), CaptureCommit::Registered);
    }
    assert!(world.effect_ids().iter().all(
        |effect| !matches!(effect.command(), Command::StopVm {epoch,..} if *epoch==plan.epoch)
    ));
    let saved = journal.lifecycle_capture_record(sandbox).unwrap().unwrap();
    assert_eq!(saved.work, accepted.work);
    assert_eq!(saved.full_capture(), accepted.full_capture());
    assert_eq!(
        saved.capture.as_ref().unwrap().reserved_material_bytes,
        budget
    );
    assert_eq!(
        saved.captured_material_bytes(),
        (stage > 0).then_some(12288)
    );
    assert!(
        super::retains_session(
            &journal.journal.connection,
            &destroy.request.binding.session
        )
        .unwrap()
    );
    assert!(
        journal
            .lifecycle_unloaded(&unload(), &v3::context(), 10)
            .unwrap()
            .is_none()
    );
    assert!(
        !journal
            .lifecycle_destroy_cleaned(destroy, &v3::context())
            .unwrap()
    );
    assert_eq!(journal.retained_resource_usage().unwrap().instances, 1);
}

#[test]
fn full_late_capture_after_destroy_keeps_material_and_never_publishes_unloaded() {
    for stage in 0..3 {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("db");
        let (mut journal, _) = ready(&path);
        let record = observed_for_full(&mut journal);
        let accepted = journal
            .admit_lifecycle_full_capture::<Evidence>(
                &record.work,
                &v3::context(),
                &full_request(&record),
                10,
                &mut NeverCrash,
            )
            .unwrap();
        let action = claim_capture_before_receipt(&mut journal, &accepted, stage);
        let mut destroy = unload();
        destroy.accepted_version = 3;
        destroy.request.binding.expected = ExpectedSession::Version(2);
        destroy.request.binding.key.sequence = 3.try_into().unwrap();
        destroy.request.operation = LifecycleOperation::Destroy {};
        journal
            .record_lifecycle_intent(&destroy, &v3::context(), &mut NeverCrash)
            .unwrap();
        let terminal_row = journal
            .world()
            .unwrap()
            .sandbox_record(record.work.anchor.local.sandbox)
            .unwrap()
            .clone();
        let budget = accepted.capture.as_ref().unwrap().reserved_material_bytes;
        journal
            .complete(
                &sandproof_protocol::WorkReceipt {
                    identity: action.identity.clone(),
                    outcome: sandproof_protocol::WorkOutcome::ObservedSuccess(FullEvidence {
                        resource: action.identity.resource.clone(),
                        command: action.command.command().clone(),
                    }),
                },
                &mut NeverCrash,
            )
            .unwrap();
        assert_late_capture_retained(&journal, &accepted, &destroy, &terminal_row, budget, stage);
        drop(journal);
        let journal = v3::open(&path);
        assert_late_capture_retained(&journal, &accepted, &destroy, &terminal_row, budget, stage);
    }
}

fn resume_request() -> AcceptedLifecycleIntent {
    let mut request = unload();
    request.accepted_version = 3;
    request.request.binding.expected = ExpectedSession::Version(2);
    request.request.binding.key.sequence = 3.try_into().unwrap();
    request.request.operation = LifecycleOperation::Resume {
        source_incarnation: 101.try_into().unwrap(),
        capture: 1.try_into().unwrap(),
        target_incarnation: 102.try_into().unwrap(),
        manifest_digest: sandproof_protocol::lifecycle::ContentDigest::from_sha256([42; 32]),
    };
    request
}

#[test]
fn recording_resume_does_not_mint_a_source_pin_or_change_native_incarnation() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("db");
    let (mut journal, _) = ready(&path);
    let source = prepare(&mut journal);
    let before = journal.world().unwrap();
    let request = resume_request();
    journal
        .record_lifecycle_intent(&request, &v3::context(), &mut NeverCrash)
        .unwrap();
    let state = journal
        .lifecycle_work_state(&request, &v3::context())
        .unwrap();
    assert_eq!(state.local_operation, None);
    assert_eq!(journal.world().unwrap(), before);
    assert_eq!(
        journal
            .lifecycle_pending_resumes(&request.request.binding.node)
            .unwrap(),
        vec![request.clone()]
    );
    assert!(matches!(
        journal.admit_lifecycle_resume(
            &request,
            &v3::context(),
            10,
            v3::limits(),
            v3::fixture::compute(),
            &mut NeverCrash
        ),
        Err(JournalError::UnresolvedAction)
    ));
    assert_eq!(journal.world().unwrap(), before);
    assert_eq!(
        journal
            .lifecycle_capture_record(state.local.unwrap().sandbox)
            .unwrap(),
        Some(source)
    );
    assert_eq!(
        journal
            .lifecycle_work_state(&request, &v3::context())
            .unwrap()
            .local_operation,
        None
    );
    drop(journal);
    let journal = v3::open(&path);
    assert_eq!(journal.world().unwrap(), before);
    assert_eq!(
        journal
            .lifecycle_work_state(&request, &v3::context())
            .unwrap()
            .local_operation,
        None
    );
}

#[test]
fn recorded_resume_rejects_changed_source_digest_and_skipped_incarnation() {
    let dir = tempfile::tempdir().unwrap();
    let (mut journal, _) = ready(&dir.path().join("db"));
    let request = resume_request();
    let mut skipped = request.clone();
    if let LifecycleOperation::Resume {
        target_incarnation, ..
    } = &mut skipped.request.operation
    {
        *target_incarnation = 103.try_into().unwrap();
    }
    assert!(
        journal
            .record_lifecycle_intent(&skipped, &v3::context(), &mut NeverCrash)
            .is_err()
    );
    journal
        .record_lifecycle_intent(&request, &v3::context(), &mut NeverCrash)
        .unwrap();
    let mut changed = request.clone();
    if let LifecycleOperation::Resume {
        manifest_digest, ..
    } = &mut changed.request.operation
    {
        *manifest_digest = sandproof_protocol::lifecycle::ContentDigest::from_sha256([43; 32]);
    }
    assert!(matches!(
        journal.record_lifecycle_intent(&changed, &v3::context(), &mut NeverCrash),
        Err(JournalError::Conflict)
    ));
    assert!(
        journal
            .record_lifecycle_intent(&request, &v3::context(), &mut NeverCrash)
            .is_ok()
    );
}

fn resume_lifetime(
    journal: &ControllerJournal,
    sandbox: sandbox_controller_core::SandboxId,
) -> crate::Lifetime {
    let world = journal.world().unwrap();
    let owner = world.sandbox_record(sandbox).unwrap().owner;
    crate::controller::admission::read_lifetime(
        &journal.journal.connection,
        &journal.namespace,
        sandbox,
        owner,
    )
    .unwrap()
    .unwrap()
}

pub(super) fn unloaded_resume_source(path: &Path) -> (ControllerJournal, SourceWorkRecord) {
    let (mut journal, _) = ready(path);
    let source = observed_for_full(&mut journal);
    let accepted = journal
        .admit_lifecycle_full_capture::<Evidence>(
            &source.work,
            &v3::context(),
            &full_request(&source),
            10,
            &mut NeverCrash,
        )
        .unwrap();
    finish_full_component(&mut journal, &accepted);
    let sandbox = source.work.anchor.local.sandbox;
    let release = journal.host_slot_context(sandbox).unwrap().unwrap();
    journal
        .release_host_slot(release, |_| Ok(()), &mut NeverCrash)
        .unwrap();
    (journal, accepted)
}

fn record_real_resume(journal: &mut ControllerJournal) -> AcceptedLifecycleIntent {
    let mut request = resume_request();
    if let LifecycleOperation::Resume {
        manifest_digest, ..
    } = &mut request.request.operation
    {
        *manifest_digest = ContentDigest::from_sha256([88; 32]);
    }
    journal
        .record_lifecycle_intent(&request, &v3::context(), &mut NeverCrash)
        .unwrap();
    request
}

fn admit_real_resume(
    journal: &mut ControllerJournal,
    request: &AcceptedLifecycleIntent,
) -> crate::LifecycleWorkState {
    journal
        .admit_lifecycle_resume(
            request,
            &v3::context(),
            10,
            v3::limits(),
            v3::fixture::compute(),
            &mut NeverCrash,
        )
        .unwrap()
}

#[test]
fn resume_sql_claim_reserves_one_successor_withdraws_old_credential_and_reopens_unchanged() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("db");
    let (mut journal, source) = unloaded_resume_source(&path);
    let sandbox = source.work.anchor.local.sandbox;
    let before = journal.world().unwrap();
    let lifetime = resume_lifetime(&journal, sandbox);
    let credential = journal.data_credential(sandbox).unwrap().unwrap();
    let request = record_real_resume(&mut journal);
    let admitted = admit_real_resume(&mut journal, &request);
    let next = journal.world().unwrap();
    let row = next.sandbox_record(sandbox).unwrap();
    assert_eq!(row.generation, 2);
    assert_eq!(row.phase, sandbox_controller_core::Phase::Starting);
    assert_eq!(resume_lifetime(&journal, sandbox), lifetime);
    assert_eq!(journal.data_credential(sandbox).unwrap(), None);
    assert_eq!(journal.lookup_data_credential(&credential.1).unwrap(), None);
    let plan = next
        .operation_plan(sandbox_controller_core::OperationKey {
            sandbox,
            operation: admitted.local_operation.unwrap(),
        })
        .unwrap();
    assert_eq!(
        plan.restore_from,
        before.sandbox_record(sandbox).unwrap().durable_snapshot
    );
    assert!(plan.vm.unwrap().0 > source.work.anchor.native.current_vm.unwrap().0);
    assert!(before.vm_record(plan.vm.unwrap()).is_none());
    assert_eq!(journal.retained_resource_usage().unwrap().instances, 1);
    assert_eq!(admit_real_resume(&mut journal, &request), admitted);
    assert_eq!(journal.world().unwrap(), next);
    drop(journal);
    let mut journal = v3::open(&path);
    assert_eq!(journal.world().unwrap(), next);
    assert_eq!(journal.data_credential(sandbox).unwrap(), None);
    assert_eq!(admit_real_resume(&mut journal, &request), admitted);
    assert_eq!(resume_lifetime(&journal, sandbox), lifetime);
}

#[test]
fn resume_unknown_sql_commit_keeps_source_claim_index_and_native_mapping_atomic() {
    for durable in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("db");
        let (mut journal, source) = unloaded_resume_source(&path);
        let sandbox = source.work.anchor.local.sandbox;
        let original = journal.data_credential(sandbox).unwrap();
        let request = record_real_resume(&mut journal);
        transaction::fail_next_commit(durable);
        assert!(matches!(
            journal.admit_lifecycle_resume(
                &request,
                &v3::context(),
                10,
                v3::limits(),
                v3::fixture::compute(),
                &mut NeverCrash
            ),
            Err(JournalError::CommitUncertain(_))
        ));
        drop(journal);
        let mut journal = v3::open(&path);
        let local = journal
            .lifecycle_work_state(&request, &v3::context())
            .unwrap();
        assert_eq!(local.local_operation.is_some(), durable);
        assert_eq!(
            journal.data_credential(sandbox).unwrap(),
            if durable { None } else { original }
        );
        assert_eq!(
            journal
                .world()
                .unwrap()
                .sandbox_record(sandbox)
                .unwrap()
                .generation,
            if durable { 2 } else { 1 }
        );
        let accepted = admit_real_resume(&mut journal, &request);
        assert!(accepted.local_operation.is_some());
        assert_eq!(
            journal
                .world()
                .unwrap()
                .sandbox_record(sandbox)
                .unwrap()
                .generation,
            2
        );
        assert_eq!(journal.data_credential(sandbox).unwrap(), None);
    }
}

#[test]
fn material_plan_question_keeps_unsealed_native_identity_without_deletion_authority() {
    let dir = tempfile::tempdir().unwrap();
    let (mut journal, _) = ready(&dir.path().join("db"));
    let source = observed_for_full(&mut journal);
    let full = full_request(&source);
    journal
        .admit_lifecycle_full_capture::<Evidence>(
            &source.work,
            &v3::context(),
            &full,
            10,
            &mut NeverCrash,
        )
        .unwrap();
    let before = journal.world().unwrap();
    let question = journal
        .lifecycle_material_retirement_request(&source.work, &v3::context())
        .unwrap();
    assert_eq!(question.sandbox(), source.work.anchor.local.sandbox);
    assert_eq!(question.generation(), source.work.anchor.native.generation);
    assert_eq!(question.full(), &full);
    assert_eq!(question.source(), source.work.requirement());
    assert_eq!(question.seal_action(), None);
    assert_eq!(question.worker(), None);
    assert_eq!(question.manifest_digest(), None);
    assert_eq!(
        question,
        journal
            .lifecycle_material_retirement_request(&source.work, &v3::context())
            .unwrap()
    );
    assert_eq!(journal.world().unwrap(), before);
    let mut changed = source.work.clone();
    changed.anchor.native.generation += 1;
    assert!(matches!(
        journal.lifecycle_material_retirement_request(&changed, &v3::context()),
        Err(JournalError::Conflict)
    ));
}

#[test]
fn resumed_store_rejects_historical_action_rebound_to_new_generation_or_epoch() {
    for changed_generation in [true, false] {
        let dir = tempfile::tempdir().unwrap();
        let (mut journal, source) = unloaded_resume_source(&dir.path().join("db"));
        let request = record_real_resume(&mut journal);
        admit_real_resume(&mut journal, &request);
        let world = journal.world().unwrap();
        let current = world
            .sandbox_record(source.work.anchor.local.sandbox)
            .unwrap();
        let (id, mut action) = {
            let mut query = journal
                .journal
                .connection
                .prepare("SELECT id,action_json FROM outbox ORDER BY id")
                .unwrap();
            let rows = query
                .query_map([], |row| {
                    Ok((row.get::<_, i64>(0)?, row.get::<_, Vec<u8>>(1)?))
                })
                .unwrap()
                .collect::<Result<Vec<_>, _>>()
                .unwrap();
            rows.into_iter()
                .map(|(id, bytes)| {
                    (
                        id,
                        serde_json::from_slice::<
                            sandproof_protocol::WorkAction<sandbox_controller_core::EffectId>,
                        >(&bytes)
                        .unwrap(),
                    )
                })
                .find(|(_, action)| {
                    if changed_generation {
                        matches!(
                            action.command.command(),
                            sandbox_controller_core::Command::InstallFence { epoch: 1, .. }
                        )
                    } else {
                        matches!(
                            action.command.command(),
                            sandbox_controller_core::Command::CleanupVm { generation: 1, .. }
                        )
                    }
                })
                .unwrap()
        };
        if changed_generation {
            action.identity.generation = current.generation;
        } else {
            action.identity.epoch = current.epoch;
        }
        journal
            .journal
            .connection
            .execute(
                "UPDATE outbox SET action_json=?1 WHERE id=?2",
                rusqlite::params![serde_json::to_vec(&action).unwrap(), id],
            )
            .unwrap();
        assert!(matches!(
            crate::controller::checkpoint::load_world(&journal.journal.connection),
            Err(JournalError::CorruptStore)
        ));
    }
}

fn claim_restore_action(
    journal: &mut ControllerJournal,
    sandbox: sandbox_controller_core::SandboxId,
    target: impl Fn(&sandbox_controller_core::Command) -> bool,
) -> sandproof_protocol::WorkAction<sandbox_controller_core::EffectId> {
    use sandbox_controller_core::workflow::{self, WorkflowStep};
    for _ in 0..32 {
        match workflow::next_step(&journal.world().unwrap(), sandbox) {
            WorkflowStep::Apply(input) => {
                journal.apply(*input, &mut NeverCrash).unwrap();
            }
            WorkflowStep::Claim => {
                let action = journal
                    .claim_next(Some(sandbox), &mut NeverCrash)
                    .unwrap()
                    .unwrap();
                if target(action.command.command()) {
                    return action;
                }
                journal
                    .complete(
                        &sandproof_protocol::WorkReceipt {
                            identity: action.identity.clone(),
                            outcome: sandproof_protocol::WorkOutcome::ObservedSuccess(
                                FullEvidence {
                                    resource: action.identity.resource.clone(),
                                    command: action.command.command().clone(),
                                },
                            ),
                        },
                        &mut NeverCrash,
                    )
                    .unwrap();
            }
            other => panic!("original restore blocked before target action: {other:?}"),
        }
    }
    panic!("bounded original restore did not reach target action");
}

#[test]
fn unknown_restore_activation_reuses_only_original_live_permit_after_fresh_bootstrap() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("db");
    let (mut journal, source) = unloaded_resume_source(&path);
    let sandbox = source.work.anchor.local.sandbox;
    let request = record_real_resume(&mut journal);
    admit_real_resume(&mut journal, &request);
    journal
        .register_data_credential(sandbox, 2, [99; 32], &mut NeverCrash)
        .unwrap();
    let work = journal
        .lifecycle_restore_work(&request, &v3::context())
        .unwrap();
    let action = claim_restore_action(&mut journal, sandbox, |command| {
        matches!(command, sandbox_controller_core::Command::ActivateVm { .. })
    });
    let permit = journal
        .lifecycle_restore_activation_permit(&work, &action, &v3::context(), 10)
        .unwrap();
    assert_eq!(permit.action(), &action);
    assert_eq!(permit.work_binding(), &work.binding_digest().unwrap());
    journal
        .complete::<FullEvidence>(
            &sandproof_protocol::WorkReceipt {
                identity: action.identity.clone(),
                outcome: sandproof_protocol::WorkOutcome::Unknown {
                    reason: sandproof_protocol::UnknownReason::ObservationUnavailable,
                    last_observation: None,
                },
            },
            &mut NeverCrash,
        )
        .unwrap();
    assert_eq!(
        journal
            .lifecycle_restore_activation_permit(&work, &action, &v3::context(), 10)
            .unwrap(),
        permit
    );
    let mut changed = action.clone();
    if let sandbox_controller_core::Command::ActivateVm { vm, .. } =
        &mut changed.command.logical.command
    {
        vm.0 += 1;
    }
    assert!(
        journal
            .lifecycle_restore_activation_permit(&work, &changed, &v3::context(), 10)
            .is_err()
    );
    let end = resume_lifetime(&journal, sandbox).deadline_ms;
    assert!(matches!(
        journal.lifecycle_restore_activation_permit(&work, &action, &v3::context(), end),
        Err(JournalError::Conflict)
    ));
    drop(journal);
    let mut journal = v3::open(&path);
    assert!(matches!(
        journal.lifecycle_restore_activation_permit(&work, &action, &v3::context(), 10),
        Err(JournalError::UnresolvedAction)
    ));
    bootstrap(&mut journal);
    assert_eq!(
        journal
            .lifecycle_restore_activation_permit(&work, &action, &v3::context(), 10)
            .unwrap(),
        permit
    );
    confirm_activation_then_claim_file_policy(&mut journal, sandbox, &action);
}

fn confirm_activation_then_claim_file_policy(
    journal: &mut ControllerJournal,
    sandbox: sandbox_controller_core::SandboxId,
    action: &sandproof_protocol::WorkAction<sandbox_controller_core::EffectId>,
) {
    assert!(
        journal
            .claim_next(Some(sandbox), &mut NeverCrash)
            .unwrap()
            .is_none()
    );
    assert!(
        journal
            .reconcile_observed(
                action,
                &sandproof_protocol::WorkReceipt {
                    identity: action.identity.clone(),
                    outcome: sandproof_protocol::WorkOutcome::ObservedSuccess(FullEvidence {
                        resource: action.identity.resource.clone(),
                        command: action.command.command().clone(),
                    }),
                },
            )
            .unwrap()
    );
    let policy = journal
        .claim_next(Some(sandbox), &mut NeverCrash)
        .unwrap()
        .expect("the original plan checks file policy after confirmed activation");
    assert!(matches!(
        policy.command.command(),
        sandbox_controller_core::Command::ApplyFilePolicy {
            sandbox: target,
            generation: 2,
            ..
        } if *target == sandbox
    ));
    assert_eq!(policy.identity.operation, action.identity.operation);
    assert_eq!(policy.identity.epoch, action.identity.epoch);
}

#[test]
fn unknown_restore_prepare_observation_does_not_mint_activation_or_ready() {
    let dir = tempfile::tempdir().unwrap();
    let (mut journal, source) = unloaded_resume_source(&dir.path().join("db"));
    let sandbox = source.work.anchor.local.sandbox;
    let request = record_real_resume(&mut journal);
    admit_real_resume(&mut journal, &request);
    journal
        .register_data_credential(sandbox, 2, [99; 32], &mut NeverCrash)
        .unwrap();
    let work = journal
        .lifecycle_restore_work(&request, &v3::context())
        .unwrap();
    let prepare = claim_restore_action(&mut journal, sandbox, |command| {
        matches!(
            command,
            sandbox_controller_core::Command::PrepareVm {
                source: Some(_),
                ..
            }
        )
    });
    journal
        .complete::<FullEvidence>(
            &sandproof_protocol::WorkReceipt {
                identity: prepare.identity.clone(),
                outcome: sandproof_protocol::WorkOutcome::Unknown {
                    reason: sandproof_protocol::UnknownReason::ObservationUnavailable,
                    last_observation: None,
                },
            },
            &mut NeverCrash,
        )
        .unwrap();
    assert!(
        journal
            .claim_next(Some(sandbox), &mut NeverCrash)
            .unwrap()
            .is_none()
    );
    // A controlled physical Prepared fact, through the original receipt path;
    // it proves neither activation nor a new execution authority.
    assert!(
        journal
            .reconcile_observed(
                &prepare,
                &sandproof_protocol::WorkReceipt {
                    identity: prepare.identity.clone(),
                    outcome: sandproof_protocol::WorkOutcome::ObservedSuccess(FullEvidence {
                        resource: prepare.identity.resource.clone(),
                        command: prepare.command.command().clone(),
                    }),
                },
            )
            .unwrap()
    );
    assert!(matches!(
        journal.lifecycle_restore_activation_permit(&work, &prepare, &v3::context(), 10),
        Err(JournalError::UnresolvedAction)
    ));
    assert!(
        !journal
            .lifecycle_ready_candidate::<FullEvidence>(&request, &v3::context(), 10)
            .unwrap()
            .can_probe()
    );
    let activate = claim_restore_action(&mut journal, sandbox, |command| {
        matches!(command, sandbox_controller_core::Command::ActivateVm { .. })
    });
    assert_eq!(
        journal
            .lifecycle_restore_activation_permit(&work, &activate, &v3::context(), 10)
            .unwrap()
            .action(),
        &activate
    );
}

fn finish_restored_component(
    journal: &mut ControllerJournal,
    sandbox: sandbox_controller_core::SandboxId,
) {
    use sandbox_controller_core::workflow::{self, WorkflowStep};
    for _ in 0..32 {
        match workflow::next_step(&journal.world().unwrap(), sandbox) {
            WorkflowStep::Apply(input) => {
                journal.apply(*input, &mut NeverCrash).unwrap();
            }
            WorkflowStep::Claim => {
                let action = journal
                    .claim_next(Some(sandbox), &mut NeverCrash)
                    .unwrap()
                    .unwrap();
                journal
                    .complete(
                        &sandproof_protocol::WorkReceipt {
                            identity: action.identity.clone(),
                            outcome: sandproof_protocol::WorkOutcome::ObservedSuccess(
                                FullEvidence {
                                    resource: action.identity.resource.clone(),
                                    command: action.command.command().clone(),
                                },
                            ),
                        },
                        &mut NeverCrash,
                    )
                    .unwrap();
            }
            WorkflowStep::PublishRunning => {
                journal
                    .publish_running_at(sandbox, 10, &mut NeverCrash)
                    .unwrap();
            }
            WorkflowStep::Complete => return,
            WorkflowStep::Blocked => panic!("original restored component blocked"),
        }
    }
    panic!("bounded original restore did not complete");
}

fn second_unload(intent: &AcceptedLifecycleIntent) -> AcceptedLifecycleIntent {
    let mut next = intent.clone();
    next.accepted_version = 4;
    next.request.binding.expected = ExpectedSession::Version(3);
    next.request.binding.key.sequence = 4.try_into().unwrap();
    next.request.operation = LifecycleOperation::Unload {
        source_incarnation: IncarnationId::try_from(102).unwrap(),
        capture: CaptureId::try_from(2).unwrap(),
    };
    next
}

fn running_successor(
    path: &Path,
) -> (ControllerJournal, SourceWorkRecord, AcceptedLifecycleIntent) {
    let (mut journal, first) = unloaded_resume_source(path);
    let sandbox = first.work.anchor.local.sandbox;
    let intent = record_real_resume(&mut journal);
    admit_real_resume(&mut journal, &intent);
    journal
        .register_data_credential(sandbox, 2, [99; 32], &mut NeverCrash)
        .unwrap();
    finish_restored_component(&mut journal, sandbox);
    assert_eq!(
        journal
            .world()
            .unwrap()
            .sandbox_record(sandbox)
            .unwrap()
            .phase,
        sandbox_controller_core::Phase::Running
    );
    (journal, first, intent)
}

#[test]
fn completed_resume_readiness_requires_all_original_fresh_startup_probes() {
    use crate::{CreateReadiness, CreateReadinessPending};
    let dir = tempfile::tempdir().unwrap();
    let (journal, _, request) = running_successor(&dir.path().join("db"));
    let candidate = journal
        .lifecycle_ready_candidate::<FullEvidence>(&request, &v3::context(), 10)
        .unwrap();
    assert!(candidate.can_probe());
    assert_eq!(candidate.actions.len(), 5);
    assert_eq!(candidate.view.request, request);
    assert_eq!(candidate.view.current_request, request);
    let local = candidate.view.local.unwrap();
    assert_eq!(local.operation, core::OperationId(3));
    let world = journal.world().unwrap();
    let row = world.sandbox_record(local.sandbox).unwrap();
    assert_eq!(row.generation, 2);
    let key = core::OperationKey {
        sandbox: local.sandbox,
        operation: local.operation,
    };
    let operation = crate::controller::identity::operation_id(&journal.namespace, key).unwrap();
    for action in &candidate.actions {
        assert_eq!(action.identity.operation, operation);
        assert_eq!(action.identity.generation, 2);
        assert_eq!(action.identity.epoch, row.epoch);
        assert_eq!(action.command.command().sandbox(), local.sandbox);
        assert_eq!(
            world
                .logical_effect(&action.command.logical)
                .unwrap()
                .origin,
            core::EffectOrigin::Operation(key)
        );
    }
    let probes: Vec<_> = candidate
        .actions
        .iter()
        .map(|action| {
            Some(sandproof_protocol::WorkReceipt {
                identity: action.identity.clone(),
                outcome: sandproof_protocol::WorkOutcome::ObservedSuccess(FullEvidence {
                    resource: action.identity.resource.clone(),
                    command: action.command.command().clone(),
                }),
            })
        })
        .collect();
    for (index, action) in candidate.actions.iter().enumerate() {
        if matches!(
            action.command.command(),
            sandbox_controller_core::Command::PrepareVm { .. }
                | sandbox_controller_core::Command::ConfigureNetwork { .. }
        ) {
            let mut missing = probes.clone();
            missing[index] = None;
            assert_eq!(
                journal
                    .finish_lifecycle_ready(&candidate, &v3::context(), &missing, true, 10)
                    .unwrap()
                    .readiness,
                CreateReadiness::Pending(CreateReadinessPending::HostUnobserved)
            );
        }
    }
    assert!(matches!(
        journal
            .finish_lifecycle_ready(&candidate, &v3::context(), &probes, false, 10)
            .unwrap()
            .readiness,
        CreateReadiness::Pending(CreateReadinessPending::CredentialUnobserved)
    ));
    assert!(matches!(
        journal
            .finish_lifecycle_ready(&candidate, &v3::context(), &probes, true, 10)
            .unwrap()
            .readiness,
        CreateReadiness::Observed(_)
    ));
}

fn assert_cross_round_source_tampering_rejected(
    journal: &ControllerJournal,
    saved: &[SourceWorkRecord],
) {
    let world = journal.world().unwrap();
    for duplicate in [false, true] {
        let tx = journal.journal.connection.unchecked_transaction().unwrap();
        let mut sources = codec::load(&tx).unwrap().unwrap();
        if duplicate {
            let key = saved[0].work.requirement.work_binding.as_bytes().as_slice();
            let bytes = serde_json::to_vec(&saved[0]).unwrap();
            assert!(tx.execute("UPDATE controller_source_work_records SET work_key=?1,state_json=?2,sha256=?3 WHERE id=2",rusqlite::params![key,bytes,sha2::Sha256::digest(&bytes).as_slice()]).is_err(), "SQL uniqueness must reject a duplicate original work key");
        } else {
            sources.records[1].capture.as_mut().unwrap().action =
                saved[0].capture.as_ref().unwrap().action.clone();
            let bytes = serde_json::to_vec(&sources.records[1]).unwrap();
            tx.execute(
                "UPDATE controller_source_work_records SET state_json=?1,sha256=?2 WHERE id=2",
                rusqlite::params![bytes, sha2::Sha256::digest(&bytes).as_slice()],
            )
            .unwrap();
            assert!(matches!(
                codec::validate(&tx, &world),
                Err(JournalError::CorruptStore)
            ));
        }
        // Dropping this deliberate corruption transaction preserves the real
        // source store and its original checksums for the next negative case.
        drop(tx);
    }
}

#[test]
fn second_full_capture_keeps_first_source_and_running_backing_bound_to_their_original_operations() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("db");
    let (mut journal, first, intent) = running_successor(&path);
    let sandbox = first.work.anchor.local.sandbox;
    let first_before = journal.lifecycle_source_work(&v3::config().node).unwrap()[0].clone();
    assert!(first_before.restore.as_ref().unwrap().material_use);
    let next = second_unload(&intent);
    journal
        .record_lifecycle_intent(&next, &v3::context(), &mut NeverCrash)
        .unwrap();
    let second = journal
        .prepare_lifecycle_source_work::<FullEvidence>(&next, &v3::context(), 10, &mut NeverCrash)
        .unwrap();
    assert_eq!(second.work.anchor.native.generation, 2);
    assert_eq!(second.work.anchor.execution_origin, intent);
    assert_ne!(
        second.work.requirement().work_binding,
        first.work.requirement().work_binding
    );
    journal
        .save_source_observation::<FullEvidence>(
            &second.work,
            &v3::context(),
            observation(second.work.requirement()),
            10,
            &mut NeverCrash,
        )
        .unwrap();
    let admitted = journal
        .admit_lifecycle_full_capture::<FullEvidence>(
            &second.work,
            &v3::context(),
            &full_request(&second),
            10,
            &mut NeverCrash,
        )
        .unwrap();
    finish_full_component(&mut journal, &admitted);
    let saved = journal.lifecycle_source_work(&v3::config().node).unwrap();
    assert_eq!(saved.len(), 2);
    assert_eq!(
        saved[0], first_before,
        "a new capture cannot clear the first restore's backing-file responsibility"
    );
    assert_eq!(saved[1].work, second.work);
    assert_ne!(saved[0].capture_operation(), saved[1].capture_operation());
    assert_eq!(
        journal.lifecycle_capture_record(sandbox).unwrap().unwrap(),
        saved[1]
    );
    assert_eq!(
        journal
            .committed_capture_expectation(&first.work)
            .unwrap()
            .unwrap()
            .work(),
        first.full_capture().unwrap()
    );
    drop(journal);
    let journal = v3::open(&path);
    assert_eq!(
        journal.lifecycle_source_work(&v3::config().node).unwrap(),
        saved
    );
    assert_cross_round_source_tampering_rejected(&journal, &saved);
}

#[test]
fn second_source_question_unknown_commit_preserves_both_original_material_responsibilities() {
    for durable in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("db");
        let (mut journal, first, intent) = running_successor(&path);
        let before = journal.lifecycle_source_work(&v3::config().node).unwrap();
        let next = second_unload(&intent);
        journal
            .record_lifecycle_intent(&next, &v3::context(), &mut NeverCrash)
            .unwrap();
        transaction::fail_next_commit(durable);
        assert!(matches!(
            journal.prepare_lifecycle_source_work::<FullEvidence>(
                &next,
                &v3::context(),
                10,
                &mut NeverCrash
            ),
            Err(JournalError::CommitUncertain(_))
        ));
        drop(journal);
        let mut journal = v3::open(&path);
        let recovered = journal.lifecycle_source_work(&v3::config().node).unwrap();
        assert_eq!(recovered.len(), 1 + usize::from(durable));
        assert_eq!(recovered[0], before[0]);
        assert!(recovered[0].restore.as_ref().unwrap().material_use);
        bootstrap(&mut journal);
        let question = journal
            .prepare_lifecycle_source_work::<FullEvidence>(
                &next,
                &v3::context(),
                10,
                &mut NeverCrash,
            )
            .unwrap();
        assert_eq!(question.work.request(), &next);
        assert_ne!(
            question.work.requirement().work_binding,
            first.work.requirement().work_binding
        );
        let replay = journal
            .prepare_lifecycle_source_work::<FullEvidence>(
                &next,
                &v3::context(),
                10,
                &mut NeverCrash,
            )
            .unwrap();
        assert_eq!(replay, question);
        assert_eq!(
            journal
                .lifecycle_source_work(&v3::config().node)
                .unwrap()
                .len(),
            2
        );
    }
}

fn bounded_source_profile(
    path: &Path,
    sessions: u16,
    max_sql_bytes: u64,
) -> (
    ControllerJournal,
    crate::LifecycleInboxConfig,
    SourceObservationConfig,
) {
    let mut maintenance = v3::maintenance();
    let cap = maintenance.capacity.as_mut().unwrap();
    cap.max_sql_value_bytes = 16 * 1024 * 1024;
    cap.max_sql_bytes = max_sql_bytes;
    let host = v3::fixture::complete_fixture::host(
        path,
        "0123456789abcdef",
        64,
        2 * 1024 * 1024,
        v3::CLEANUP_BYTES + 1000,
    );
    let journal =
        ControllerJournal::open_complete(path, "0123456789abcdef", host, maintenance, 0).unwrap();
    let mut inbox = v3::config();
    inbox.max_sessions = sessions;
    let mut source = config();
    source.max_work_bytes = 65536;
    source.max_configuration_bytes = 65536;
    (journal, inbox, source)
}

#[test]
fn source_gc_plan_budget_is_reserved_per_work_for_six_sessions_without_a_global_value_limit() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("db");
    let (mut journal, inbox, source) = bounded_source_profile(&path, 6, 64 * 1024 * 1024);
    let per_row = source.required_bytes(1);
    let rows = 6 * (8 - 2);
    assert!(8192 + rows * per_row > 16 * 1024 * 1024);
    assert!(per_row < 16 * 1024 * 1024);
    let inbox_bytes = inbox.required_bytes();
    let before = crate::controller::capacity::fixture_sql_budget(
        &journal.journal.connection,
        &journal.world().unwrap(),
    )
    .unwrap();
    journal
        .initialize_lifecycle_inbox(inbox, source, &mut NeverCrash)
        .unwrap();
    let after = crate::controller::capacity::fixture_sql_budget(
        &journal.journal.connection,
        &journal.world().unwrap(),
    )
    .unwrap();
    assert_eq!(after.0 - before.0, rows + 2);
    assert_eq!(
        after.1 - before.1,
        inbox_bytes + 48 + 8192 + 48 + rows * (per_row + 80)
    );
    let (actual,sum,small,large):(i64,i64,i64,i64)=journal.journal.connection.query_row("SELECT count(*),sum(reserve_bytes),min(reserve_bytes),max(reserve_bytes) FROM controller_source_work_records",[],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).unwrap();
    assert_eq!(u64::try_from(actual).unwrap(), rows);
    assert_eq!(u64::try_from(sum).unwrap(), rows * per_row);
    assert_eq!(u64::try_from(small).unwrap(), per_row);
    assert_eq!(small, large);
    let max_plan = vec![255_u8; crate::capture::MATERIAL_RETIREMENT_PLAN_MAX_BYTES];
    assert_eq!(serde_json::to_vec(&max_plan).unwrap().len(), 65_537);
    assert!(
        (serde_json::to_vec(&max_plan).unwrap().len() as u64)
            < crate::capture::MATERIAL_RETIREMENT_RECORD_MAX_BYTES
    );
    assert!(journal.world().unwrap().sandbox_records().is_empty());
    drop(journal);
    let (journal, _, _) = bounded_source_profile(&path, 6, 64 * 1024 * 1024);
    assert!(
        codec::load(&journal.journal.connection)
            .unwrap()
            .unwrap()
            .records
            .is_empty()
    );
}

#[test]
fn source_gc_total_preallocation_one_byte_short_rolls_back_all_rows_and_inbox() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("db");
    let mut source = config();
    source.max_work_bytes = 65536;
    source.max_configuration_bytes = 65536;
    let rows = 10 * (8 - 2);
    let required = 8192 + 48 + rows * (source.required_bytes(1) + 80);
    let (mut journal, inbox, source) = bounded_source_profile(&path, 10, required - 1);
    assert!(matches!(
        journal.initialize_lifecycle_inbox(inbox, source, &mut NeverCrash),
        Err(JournalError::MaintenanceCapacity)
    ));
    assert!(journal.lifecycle_inbox_configuration().unwrap().is_none());
    let tables:i64=journal.journal.connection.query_row("SELECT count(*) FROM sqlite_master WHERE name IN ('controller_source_work','controller_source_work_records','controller_lifecycle_inbox')",[],|r|r.get(0)).unwrap();
    assert_eq!(tables, 0);
    assert!(journal.world().unwrap().sandbox_records().is_empty());
}

#[test]
fn source_work_rows_reject_missing_slots_hash_changes_and_single_row_overflow() {
    use sha2::{Digest, Sha256};
    for mode in 0..3 {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("db");
        let (mut journal, inbox, source) = bounded_source_profile(&path, 6, 64 * 1024 * 1024);
        let per_row = source.required_bytes(1);
        journal
            .initialize_lifecycle_inbox(inbox, source, &mut NeverCrash)
            .unwrap();
        match mode {
            0 => {
                journal
                    .journal
                    .connection
                    .execute("DELETE FROM controller_source_work_records WHERE id=2", [])
                    .unwrap();
            }
            1 => {
                journal
                    .journal
                    .connection
                    .execute(
                        "UPDATE controller_source_work_records SET sha256=zeroblob(32) WHERE id=1",
                        [],
                    )
                    .unwrap();
            }
            _ => {
                let bytes = vec![b'0'; usize::try_from(per_row + 1).unwrap()];
                journal.journal.connection.execute("UPDATE controller_source_work_records SET state_json=?1,sha256=?2 WHERE id=1",rusqlite::params![bytes,Sha256::digest(&bytes).as_slice()]).unwrap();
            }
        }
        assert!(matches!(
            codec::load(&journal.journal.connection),
            Err(JournalError::CorruptStore)
        ));
    }
}

#[test]
fn original_work_key_and_payload_are_already_charged_before_source_admission() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("db");
    let (mut journal, _) = ready(&path);
    let before = crate::controller::capacity::fixture_sql_budget(
        &journal.journal.connection,
        &journal.world().unwrap(),
    )
    .unwrap();
    let record = prepare(&mut journal);
    let after = crate::controller::capacity::fixture_sql_budget(
        &journal.journal.connection,
        &journal.world().unwrap(),
    )
    .unwrap();
    assert_eq!((after.0, after.1), (before.0, before.1));
    let key: Vec<u8> = journal
        .journal
        .connection
        .query_row(
            "SELECT work_key FROM controller_source_work_records WHERE id=1",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(key, record.work.requirement.work_binding.as_bytes());
}

#[test]
fn public_unload_observation_keeps_pending_and_persisted_unloaded_mutually_exclusive() {
    use sandproof_protocol::lifecycle::UnloadOutcome;
    for complete in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("db");
        let journal = if complete {
            unloaded_resume_source(&path).0
        } else {
            ready(&path).0
        };
        drop(journal);
        let state = Arc::new(Mutex::new(State::default()));
        let app = v3::application(&path, state, false);
        let observed = app
            .observe_lifecycle_unloaded(&unload(), &v3::context())
            .unwrap();
        assert!(observed.matches_request(&unload()));
        if complete {
            let UnloadOutcome::Unloaded { facts } = observed.outcome else {
                panic!("persisted release and material must yield Unloaded");
            };
            assert_eq!(
                facts.source_incarnation,
                IncarnationId::try_from(101).unwrap()
            );
            assert_eq!(facts.capture, CaptureId::try_from(1).unwrap());
        } else {
            assert_eq!(observed.outcome, UnloadOutcome::Pending);
        }
    }
}

pub(super) fn sealed_before_end(
    path: &Path,
) -> (ControllerJournal, Arc<Mutex<State>>, SourceWorkRecord) {
    let (mut journal, state) = ready(path);
    let source = observed_for_full(&mut journal);
    let accepted = journal
        .admit_lifecycle_full_capture::<Evidence>(
            &source.work,
            &v3::context(),
            &full_request(&source),
            10,
            &mut NeverCrash,
        )
        .unwrap();
    let seal = claim_full_before_worker_release(&mut journal, source.work.anchor.local.sandbox);
    let worker = crate::capture::CaptureWorkerBinding::new(
        SourceProcessIdentity {
            pid: 2345,
            start_time_ticks: 9876,
            boot_id: "fixture-boot".into(),
        },
        ContentDigest::from_sha256([9; 32]),
    )
    .unwrap();
    journal
        .authorize_lifecycle_full_capture(
            &source.work,
            &v3::context(),
            &seal,
            worker,
            10,
            &mut NeverCrash,
        )
        .unwrap();
    journal
        .complete(
            &sandproof_protocol::WorkReceipt {
                identity: seal.identity.clone(),
                outcome: sandproof_protocol::WorkOutcome::ObservedSuccess(FullEvidence {
                    resource: seal.identity.resource.clone(),
                    command: seal.command.command().clone(),
                }),
            },
            &mut NeverCrash,
        )
        .unwrap();
    (journal, state, accepted)
}

pub(super) fn actual_api_end(
    record: &SourceWorkRecord,
) -> crate::capture::CaptureRequestEndObservation {
    let capture = record.capture.as_ref().unwrap();
    crate::capture::CaptureRequestEndObservation {
        action: capture.action.clone().unwrap(),
        full_work_binding: capture.full.binding_digest().unwrap(),
        worker: capture.worker.clone().unwrap(),
        source: record.work.requirement().source.clone(),
        outcome: crate::capture::CaptureRequestEndOutcome::Returned { http_status: 204 },
    }
}

#[test]
fn original_api_end_fact_is_durable_but_changes_no_native_or_material_responsibility() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("db");
    let (mut journal, _state, source) = sealed_before_end(&path);
    let saved = journal.lifecycle_source_work(&v3::config().node).unwrap()[0].clone();
    let before = journal.world().unwrap();
    let observation = actual_api_end(&saved);
    let receipt = journal
        .record_lifecycle_capture_request_end(
            &source.work,
            &v3::context(),
            observation.clone(),
            &mut NeverCrash,
        )
        .unwrap();
    let core::capture::CaptureWorkEnd::Returned { seal } = receipt.end else {
        panic!("a returned API end cannot become a never-released fact");
    };
    let action = saved.capture.as_ref().unwrap().action.as_ref().unwrap();
    assert_eq!(seal.attempt, action.command.attempt);
    assert_eq!(
        seal.command(core::capture::OriginalContinueIdentity {
            key: core::OperationKey {
                sandbox: source.work.anchor.local.sandbox,
                operation: saved.capture_operation().unwrap(),
            },
            epoch: action.identity.epoch,
            generation: action.identity.generation,
            vm: source.work.anchor.native.current_vm.unwrap(),
        }),
        *action.command.command()
    );
    assert_eq!(
        receipt.work_binding,
        *saved
            .full_capture()
            .unwrap()
            .binding_digest()
            .unwrap()
            .as_bytes()
    );
    assert_eq!(journal.world().unwrap(), before);
    let after = journal.lifecycle_source_work(&v3::config().node).unwrap()[0].clone();
    assert_eq!(after.captured_manifest(), saved.captured_manifest());
    assert_eq!(
        after.captured_material_bytes(),
        saved.captured_material_bytes()
    );
    assert_eq!(
        after.capture.as_ref().unwrap().reserved_material_bytes,
        saved.capture.as_ref().unwrap().reserved_material_bytes
    );
    assert_eq!(after.capture_work_end_receipt().unwrap(), Some(receipt));
    assert_eq!(
        journal
            .record_lifecycle_capture_request_end(
                &source.work,
                &v3::context(),
                observation,
                &mut NeverCrash
            )
            .unwrap(),
        receipt
    );
    drop(journal);
    let journal = v3::open(&path);
    assert_eq!(
        journal.lifecycle_source_work(&v3::config().node).unwrap()[0]
            .capture_work_end_receipt()
            .unwrap(),
        Some(receipt)
    );
}

#[test]
fn original_api_end_rejects_substituted_source_action_worker_digest_and_incomplete_status() {
    let dir = tempfile::tempdir().unwrap();
    let (mut journal, _state, source) = sealed_before_end(&dir.path().join("db"));
    let saved = journal.lifecycle_source_work(&v3::config().node).unwrap()[0].clone();
    let before = journal.world().unwrap();
    for mode in 0..5 {
        let mut end = actual_api_end(&saved);
        match mode {
            0 => end.source.pid += 1,
            1 => end.action.identity.generation += 1,
            2 => end.full_work_binding = ContentDigest::from_sha256([0; 32]),
            3 => {
                end.outcome =
                    crate::capture::CaptureRequestEndOutcome::Returned { http_status: 100 }
            }
            _ => {
                end.worker = crate::capture::CaptureWorkerBinding::new(
                    SourceProcessIdentity {
                        pid: 98765,
                        start_time_ticks: 33,
                        boot_id: "fixture-boot".into(),
                    },
                    ContentDigest::from_sha256([9; 32]),
                )
                .unwrap();
            }
        }
        assert!(matches!(
            journal.record_lifecycle_capture_request_end(
                &source.work,
                &v3::context(),
                end,
                &mut NeverCrash
            ),
            Err(JournalError::InvalidHostEvidence)
        ));
        assert_eq!(journal.world().unwrap(), before);
        assert!(
            journal.lifecycle_source_work(&v3::config().node).unwrap()[0]
                .capture_work_end_receipt()
                .unwrap()
                .is_none()
        );
    }
    assert!(matches!(
        journal.record_lifecycle_capture_never_released(
            &source.work,
            &v3::context(),
            &mut NeverCrash
        ),
        Err(JournalError::UnresolvedAction)
    ));
}

#[test]
fn original_api_end_unknown_commit_reopens_the_exact_saved_or_absent_fact() {
    for durable in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("db");
        let (mut journal, _state, source) = sealed_before_end(&path);
        let saved = journal.lifecycle_source_work(&v3::config().node).unwrap()[0].clone();
        let observation = actual_api_end(&saved);
        transaction::fail_next_commit(durable);
        assert!(matches!(
            journal.record_lifecycle_capture_request_end(
                &source.work,
                &v3::context(),
                observation.clone(),
                &mut NeverCrash
            ),
            Err(JournalError::CommitUncertain(_))
        ));
        drop(journal);
        let mut journal = v3::open(&path);
        let recovered = journal.lifecycle_source_work(&v3::config().node).unwrap()[0]
            .capture_work_end_receipt()
            .unwrap();
        assert_eq!(recovered.is_some(), durable);
        let retried = journal
            .record_lifecycle_capture_request_end(
                &source.work,
                &v3::context(),
                observation,
                &mut NeverCrash,
            )
            .unwrap();
        if let Some(receipt) = recovered {
            assert_eq!(retried, receipt);
        }
        assert_eq!(
            journal
                .lifecycle_source_work(&v3::config().node)
                .unwrap()
                .len(),
            1
        );
    }
}

#[test]
fn never_released_full_fact_requires_revocation_and_cannot_invent_an_api_end() {
    use sandbox_controller_core as core;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("db");
    let (mut journal, _) = ready(&path);
    let source = observed_for_full(&mut journal);
    let admitted = journal
        .admit_lifecycle_full_capture::<Evidence>(
            &source.work,
            &v3::context(),
            &full_request(&source),
            10,
            &mut NeverCrash,
        )
        .unwrap();
    let sandbox = source.work.anchor.local.sandbox;
    assert!(matches!(
        journal.record_lifecycle_capture_never_released(
            &source.work,
            &v3::context(),
            &mut NeverCrash
        ),
        Err(JournalError::UnresolvedAction)
    ));
    journal
        .apply(
            core::ControllerInput::Request(core::Request::Compensate(
                core::request::CompensationRequest {
                    sandbox,
                    failure: core::FailureKind::Cancelled,
                    evidence: None,
                },
            )),
            &mut NeverCrash,
        )
        .unwrap();
    let before = journal.world().unwrap();
    let receipt = journal
        .record_lifecycle_capture_never_released(&source.work, &v3::context(), &mut NeverCrash)
        .unwrap();
    assert_eq!(journal.world().unwrap(), before);
    let work = journal
        .lifecycle_original_continue_work(&source.work, &v3::context())
        .unwrap();
    assert_eq!(work.end_receipt(), receipt);
    assert_eq!(
        work.identity().key.operation,
        admitted.capture_operation().unwrap()
    );
    assert_eq!(
        work.identity().vm,
        source.work.anchor.native.current_vm.unwrap()
    );
    assert_eq!(work.full(), &full_request(&source));
    let world = journal.world().unwrap();
    let native = world.sandbox_record(sandbox).unwrap();
    assert_eq!(work.network_policy(), &native.policy);
    assert_eq!(work.file_policy(), native.file_policy);
    assert_eq!(
        receipt.work_binding,
        *work.full().binding_digest().unwrap().as_bytes()
    );
    let stored = journal.lifecycle_source_work(&v3::config().node).unwrap()[0].clone();
    assert!(stored.capture.as_ref().unwrap().worker.is_none());
    assert!(stored.capture.as_ref().unwrap().completed.is_none());
    drop(journal);
    let mut journal = v3::open(&path);
    assert_eq!(
        journal
            .record_lifecycle_capture_never_released(&source.work, &v3::context(), &mut NeverCrash)
            .unwrap(),
        receipt
    );
    assert_eq!(
        journal
            .lifecycle_original_continue_work(&source.work, &v3::context())
            .unwrap(),
        work
    );
}

fn claim_full_before_worker_release(
    journal: &mut ControllerJournal,
    sandbox: sandbox_controller_core::SandboxId,
) -> sandproof_protocol::WorkAction<sandbox_controller_core::EffectId> {
    use sandbox_controller_core::workflow::{self, WorkflowStep};
    for _ in 0..32 {
        match workflow::next_step(&journal.world().unwrap(), sandbox) {
            WorkflowStep::Apply(input) => {
                journal.apply(*input, &mut NeverCrash).unwrap();
            }
            WorkflowStep::Claim => {
                let action = journal
                    .claim_next(Some(sandbox), &mut NeverCrash)
                    .unwrap()
                    .unwrap();
                if matches!(
                    action.command.command(),
                    sandbox_controller_core::Command::SealSnapshot { .. }
                ) {
                    return action;
                }
                journal
                    .complete(
                        &sandproof_protocol::WorkReceipt {
                            identity: action.identity.clone(),
                            outcome: sandproof_protocol::WorkOutcome::ObservedSuccess(
                                FullEvidence {
                                    resource: action.identity.resource.clone(),
                                    command: action.command.command().clone(),
                                },
                            ),
                        },
                        &mut NeverCrash,
                    )
                    .unwrap();
            }
            other => panic!("original Full did not reach closed worker gate: {other:?}"),
        }
    }
    panic!("bounded original Full did not reach worker gate");
}

#[test]
fn late_full_worker_authorization_is_rejected_after_native_revocation_before_and_after_end_record()
{
    use sandbox_controller_core as core;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("db");
    let (mut journal, _) = ready(&path);
    let source = observed_for_full(&mut journal);
    journal
        .admit_lifecycle_full_capture::<Evidence>(
            &source.work,
            &v3::context(),
            &full_request(&source),
            10,
            &mut NeverCrash,
        )
        .unwrap();
    let sandbox = source.work.anchor.local.sandbox;
    let action = claim_full_before_worker_release(&mut journal, sandbox);
    let cancel = core::ControllerInput::Request(core::Request::Compensate(
        core::request::CompensationRequest {
            sandbox,
            failure: core::FailureKind::Cancelled,
            evidence: None,
        },
    ));
    // The public coordinator still rejects cancellation with an unresolved
    // dispatch. Exercise the guard's native-revoked input separately through
    // the ORIGINAL private reducer transaction; do not weaken that public gate.
    assert!(matches!(
        journal.apply(cancel.clone(), &mut NeverCrash),
        Err(JournalError::UnresolvedAction)
    ));
    let namespace = journal.namespace.clone();
    let tx = crate::controller::transaction::WorldTransaction::begin(
        &mut journal.journal.connection,
        &journal.committed_world,
    )
    .unwrap();
    crate::controller::storage::apply(&tx, &namespace, cancel).unwrap();
    tx.commit().unwrap();
    let worker = crate::capture::CaptureWorkerBinding::new(
        SourceProcessIdentity {
            pid: 2345,
            start_time_ticks: 9876,
            boot_id: "fixture-boot".into(),
        },
        ContentDigest::from_sha256([9; 32]),
    )
    .unwrap();
    for end_saved in [false, true] {
        if end_saved {
            journal
                .record_lifecycle_capture_never_released(
                    &source.work,
                    &v3::context(),
                    &mut NeverCrash,
                )
                .unwrap();
        }
        let before = journal.world().unwrap();
        assert!(matches!(
            journal.authorize_lifecycle_full_capture(
                &source.work,
                &v3::context(),
                &action,
                worker.clone(),
                10,
                &mut NeverCrash
            ),
            Err(JournalError::Conflict)
        ));
        assert_eq!(journal.world().unwrap(), before);
        assert!(
            journal.lifecycle_source_work(&v3::config().node).unwrap()[0]
                .capture_worker()
                .is_none()
        );
    }
}

#[test]
fn resume_slot_installation_ignores_only_settled_prior_incarnation_work() {
    use crate::application::can_install_host_slot_binding;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("db");
    let (mut journal, source) = unloaded_resume_source(&path);
    let sandbox = source.work.anchor.local.sandbox;
    let request = record_real_resume(&mut journal);
    admit_real_resume(&mut journal, &request);
    let world = journal.world().unwrap();
    assert_eq!(world.sandbox_record(sandbox).unwrap().generation, 2);
    assert!(world.effect_ids().iter().any(|effect| matches!(
        world.effect_status(effect),
        Some(sandbox_controller_core::EffectStatus::Acknowledged(_))
    )));
    assert!(
        can_install_host_slot_binding(&world, sandbox),
        "settled prior incarnation must not prevent a never-dispatched successor binding"
    );
    drop(journal);
    let mut journal = v3::open(&path);
    assert!(can_install_host_slot_binding(
        &journal.world().unwrap(),
        sandbox
    ));
    bootstrap(&mut journal);
    let mut first_action = None;
    for _ in 0..8 {
        match sandbox_controller_core::workflow::next_step(&journal.world().unwrap(), sandbox) {
            sandbox_controller_core::workflow::WorkflowStep::Apply(input) => {
                journal.apply(*input, &mut NeverCrash).unwrap();
            }
            sandbox_controller_core::workflow::WorkflowStep::Claim => {
                assert!(can_install_host_slot_binding(
                    &journal.world().unwrap(),
                    sandbox
                ));
                first_action = journal.claim_next(Some(sandbox), &mut NeverCrash).unwrap();
                break;
            }
            other => {
                panic!("original resume unexpectedly blocked before its first claim: {other:?}")
            }
        }
    }
    let action = first_action.expect("original workflow must reach its first host claim");
    assert_eq!(action.identity.generation, 2);
    let claimed = journal.world().unwrap();
    assert!(!can_install_host_slot_binding(&claimed, sandbox));
    let unknown = sandbox_controller_core::handle_controller(
        &claimed,
        sandbox_controller_core::ControllerInput::Effect(
            sandbox_controller_core::EffectInput::Timeout(action.command),
        ),
    )
    .unwrap()
    .next;
    assert!(!can_install_host_slot_binding(&unknown, sandbox));
}
