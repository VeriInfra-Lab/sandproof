use super::*;
use crate::controller::{tests_support::lifecycle_application as v3, transaction};
use crate::{NeverCrash, application::WorkEvidence, capture::HostEpochObservation};
use sandproof_protocol::lifecycle::{
    CaptureId, ExpectedSession, IncarnationId, LifecycleOperation,
};
use std::{
    path::Path,
    sync::{Arc, Mutex},
};
use v3::fixture::{State, TestEpoch, TestHost};
type Evidence = <TestHost as WorkEvidence>::Evidence;

fn config() -> SourceObservationConfig {
    SourceObservationConfig {
        max_work_bytes: 32_768,
        max_configuration_bytes: 1024,
        max_executable_bytes: 1024 * 1024,
        max_kernel_bytes: 1024 * 1024,
        max_disk_bytes: 1024 * 1024,
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
            "SELECT reserve_bytes FROM controller_source_work",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(u64::try_from(reserve).unwrap(), config().required_bytes(2));
    drop(journal);
    v3::open(&path)
        .check_lifecycle_source_observations(&config())
        .unwrap();
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
        for table in ["controller_lifecycle_inbox", "controller_source_work"] {
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
                        crate::controller::tests_support::limits(),
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
                let mut saved=codec::load(&db).unwrap().unwrap();
                match mode {
                    5=>saved.node.node_generation=2.try_into().unwrap(),
                    6=>{saved.records[0].work.anchor.native.current_vm=Some(sandbox_controller_core::VmId(12345));saved.records[0].work.requirement.work_binding=digest(&serde_json::to_vec(&saved.records[0].work.anchor).unwrap());}
                    _=>saved.records[0].observation.as_mut().unwrap().configuration=vec![0;1025],
                }
                let bytes=serde_json::to_vec(&saved).unwrap();db.execute("UPDATE controller_source_work SET state_json=?1,sha256=?2",rusqlite::params![bytes,Sha256::digest(&bytes).as_slice()]).unwrap();
            }
        }
        drop(db);
        let reserve = v3::fixture::complete_fixture::CLEANUP_BYTES + 1000;
        let maintenance = v3::fixture::complete_fixture::maintenance();
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
    for version in [0, 17, 19] {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("db");
        let (journal, _) = running_source(&path);
        drop(journal);
        let conn = rusqlite::Connection::open(&path).unwrap();
        conn.execute("UPDATE controller_meta SET version=?1", [version])
            .unwrap();
        drop(conn);
        let reserve = v3::fixture::complete_fixture::CLEANUP_BYTES + 1000;
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
                &v3::fixture::complete_fixture::maintenance()
            )
            .is_err()
        );
        assert!(
            ControllerJournal::open_complete(
                &path,
                "0123456789abcdef",
                host,
                v3::fixture::complete_fixture::maintenance(),
                0
            )
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
    for table in ["controller_lifecycle_inbox", "controller_source_work"] {
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
