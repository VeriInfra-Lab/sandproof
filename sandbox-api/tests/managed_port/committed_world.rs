//! Real `SQLite` commit boundaries and SQL-only lifetime authority with a warm
//! controller replay cache. Host actions remain the controlled integration fake.
use super::*;
use sandproof_api::SandboxSession;
use sandproof_controller_service::application::DataKind;
use sandproof_controller_service::{AdmissionError, CrashInjector, CrashPoint, JournalError};
use std::path::Path;

const DATA_KINDS: [DataKind; 3] = [DataKind::Exec, DataKind::FileRead, DataKind::FileWrite];

fn database(directory: &TempDir) -> rusqlite::Connection {
    rusqlite::Connection::open_with_flags(
        directory.path().join("controller.db"),
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .unwrap()
}

fn input_tail(connection: &rusqlite::Connection) -> (i64, i64) {
    connection
        .query_row(
            "SELECT count(*), coalesce(max(sequence), 0) FROM controller_inputs",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap()
}

fn deadline(connection: &rusqlite::Connection, sandbox: &str) -> i64 {
    connection
        .query_row(
            "SELECT deadline_ms FROM sandbox_lifetimes WHERE sandbox_id=?1",
            [sandbox],
            |row| row.get(0),
        )
        .unwrap()
}

fn restore_host(sessions: &[SandboxSession], stops: Arc<AtomicUsize>) -> FakeHost {
    let host = FakeHost {
        stop_calls: Some(stops),
        ..FakeHost::default()
    };
    for (index, session) in sessions.iter().enumerate() {
        host.put_secret(
            core::SandboxId(u64::try_from(index + 1).unwrap()),
            &session.envd_access_token,
        )
        .unwrap();
    }
    host
}

#[derive(Clone, Copy, Debug)]
enum CommitFailure {
    Kill,
    Lifetime,
}

fn install_commit_failure(path: &Path, failure: CommitFailure) {
    let connection = rusqlite::Connection::open(path).unwrap();
    // The statement succeeds; only COMMIT checks this deferred constraint.
    // Install before reopening the journal so this is not an external-writer
    // identity failure. No invalid controller input is ever committed.
    connection
        .execute_batch(
            "CREATE TABLE commit_fault_parent (id INTEGER PRIMARY KEY);
             CREATE TABLE commit_fault_child (
                 parent INTEGER REFERENCES commit_fault_parent(id)
                     DEFERRABLE INITIALLY DEFERRED
             );",
        )
        .unwrap();
    let target = match failure {
        CommitFailure::Kill => "INSERT ON controller_inputs",
        CommitFailure::Lifetime => "UPDATE OF deadline_ms ON sandbox_lifetimes",
    };
    connection
        .execute_batch(&format!(
            "CREATE TRIGGER reject_commit AFTER {target} BEGIN
                 INSERT INTO commit_fault_child(parent) VALUES (1);
             END;"
        ))
        .unwrap();
}

#[test]
fn uncertain_kill_revokes_warm_routes_but_reauthorizes_only_rolled_back_authority() {
    assert_uncertain_revocation(CommitFailure::Kill);
}

#[test]
fn uncertain_sql_only_timeout_revokes_warm_routes_without_publishing_a_deadline() {
    assert_uncertain_revocation(CommitFailure::Lifetime);
}

fn assert_uncertain_revocation(failure: CommitFailure) {
    let (directory, port) = setup();
    let sessions = [
        port.create("1", request()).unwrap(),
        port.create("2", request()).unwrap(),
    ];
    drop(port);
    let path = directory.path().join("controller.db");
    install_commit_failure(&path, failure);
    let stops = Arc::new(AtomicUsize::new(0));
    let journal = complete_fixture::open(&path, NAMESPACE).unwrap();
    let committed = journal.world().unwrap();
    let port = new_port(
        journal,
        restore_host(&sessions, Arc::clone(&stops)),
        config(),
    );
    let first = &sessions[0];
    let routes = DATA_KINDS.map(|kind| {
        port.application()
            .data_permit(&first.envd_access_token, kind)
            .unwrap()
    });
    let other = port
        .application()
        .data_permit(&sessions[1].envd_access_token, DataKind::Exec)
        .unwrap();
    let connection = database(&directory);
    let tail = input_tail(&connection);
    let original_deadline = deadline(&connection, &first.sandbox_id);
    let operation = || match failure {
        CommitFailure::Kill => port.kill("1", &first.sandbox_id),
        CommitFailure::Lifetime => port.set_timeout("1", &first.sandbox_id, 0),
    };
    assert_eq!(operation(), Err(PortError::OutcomeUnknown), "{failure:?}");
    assert!(routes.iter().all(|route| *route.revocation.borrow()));
    assert!(!*other.revocation.borrow());
    assert_eq!(stops.load(Ordering::SeqCst), 0);
    assert_eq!(input_tail(&connection), tail);
    assert_eq!(deadline(&connection, &first.sandbox_id), original_deadline);
    assert_eq!(
        connection
            .query_row("SELECT count(*) FROM commit_fault_child", [], |row| {
                row.get::<_, i64>(0)
            })
            .unwrap(),
        0
    );

    // COMMIT failed and SQLite rolled back. This same journal must reload its
    // durable Running state rather than expose the uncommitted Kill candidate.
    // Newly authorized routes get a new channel, never revive the old channel.
    let fresh = DATA_KINDS.map(|kind| {
        port.application()
            .data_permit(&first.envd_access_token, kind)
            .unwrap()
    });
    for (new, old) in fresh.iter().zip(&routes) {
        assert_eq!(new.target, old.target);
        assert!(!*new.revocation.borrow());
        assert!(*old.revocation.borrow());
    }
    assert!(port.get("1", &first.sandbox_id).is_ok());
    assert_eq!(operation(), Err(PortError::OutcomeUnknown));
    assert!(fresh.iter().all(|route| *route.revocation.borrow()));
    assert!(!*other.revocation.borrow());
    assert_eq!(stops.load(Ordering::SeqCst), 0);
    assert_eq!(input_tail(&connection), tail);
    drop(port);
    let journal = complete_fixture::open(&path, NAMESPACE).unwrap();
    assert_eq!(journal.world().unwrap(), committed);
    assert!(journal.uncertain_actions().unwrap().is_empty());
}

#[test]
fn uncertain_expiry_retains_sql_denial_and_never_dispatches_an_uncommitted_stop() {
    let (directory, port) = setup();
    let sessions = [
        port.create("1", request()).unwrap(),
        port.create("2", request()).unwrap(),
    ];
    drop(port);
    let path = directory.path().join("controller.db");
    install_commit_failure(&path, CommitFailure::Kill);
    let stops = Arc::new(AtomicUsize::new(0));
    let journal = complete_fixture::open(&path, NAMESPACE).unwrap();
    let committed = journal.world().unwrap();
    let port = new_port(
        journal,
        restore_host(&sessions, Arc::clone(&stops)),
        config(),
    );
    let first = &sessions[0];
    let routes = DATA_KINDS.map(|kind| {
        port.application()
            .data_permit(&first.envd_access_token, kind)
            .unwrap()
    });
    let other = port
        .application()
        .data_permit(&sessions[1].envd_access_token, DataKind::Exec)
        .unwrap();
    // The API has no injectable clock. Expire immediately, then prove a failed
    // expiry acceptance cannot resurrect these already-revoked subscriptions.
    port.set_timeout("1", &first.sandbox_id, 0).unwrap();
    let connection = database(&directory);
    let tail = input_tail(&connection);
    let due = deadline(&connection, &first.sandbox_id);
    for _ in 0..2 {
        assert_eq!(
            port.application().expire_due_report(),
            Err(PortError::OutcomeUnknown)
        );
        assert_eq!(stops.load(Ordering::SeqCst), 0);
        assert_eq!(input_tail(&connection), tail);
        assert_eq!(deadline(&connection, &first.sandbox_id), due);
        assert!(routes.iter().all(|route| *route.revocation.borrow()));
        assert!(!*other.revocation.borrow());
        for kind in DATA_KINDS {
            assert_eq!(
                port.application()
                    .data_permit(&first.envd_access_token, kind)
                    .unwrap_err(),
                PortError::NotFound
            );
        }
        assert!(port.get("2", &sessions[1].sandbox_id).is_ok());
    }
    drop(port);
    let journal = complete_fixture::open(&path, NAMESPACE).unwrap();
    assert_eq!(journal.world().unwrap(), committed);
    assert_eq!(
        journal.due_lifetimes(due).unwrap(),
        vec![core::SandboxId(1)]
    );
    assert!(journal.uncertain_actions().unwrap().is_empty());
}

#[test]
fn committed_sql_only_expiry_denies_new_permits_with_unchanged_input_history() {
    let (directory, port) = setup();
    let first = port.create("1", request()).unwrap();
    let other = port.create("2", request()).unwrap();
    let routes = DATA_KINDS.map(|kind| {
        port.application()
            .data_permit(&first.envd_access_token, kind)
            .unwrap()
    });
    let other_route = port
        .application()
        .data_permit(&other.envd_access_token, DataKind::Exec)
        .unwrap();
    let connection = database(&directory);
    let tail = input_tail(&connection);
    let old_deadline = deadline(&connection, &first.sandbox_id);
    port.set_timeout("1", &first.sandbox_id, 0).unwrap();
    assert_eq!(input_tail(&connection), tail);
    assert!(deadline(&connection, &first.sandbox_id) < old_deadline);
    assert!(routes.iter().all(|route| *route.revocation.borrow()));
    assert!(!*other_route.revocation.borrow());
    for kind in DATA_KINDS {
        assert_eq!(
            port.application()
                .data_permit(&first.envd_access_token, kind)
                .unwrap_err(),
            PortError::NotFound
        );
    }
    assert_eq!(port.get("1", &first.sandbox_id), Err(PortError::NotFound));
    assert_eq!(port.application().expire_due().unwrap(), 1);
    assert!(!*other_route.revocation.borrow());
    port.kill("2", &other.sandbox_id).unwrap();
}

#[test]
fn sql_only_connect_extension_preserves_routes_and_rechecks_old_expiry_hint() {
    let (directory, port) = setup();
    let first = port.create("1", request()).unwrap();
    let routes = DATA_KINDS.map(|kind| {
        port.application()
            .data_permit(&first.envd_access_token, kind)
            .unwrap()
    });
    let connection = database(&directory);
    let tail = input_tail(&connection);
    let old_deadline = deadline(&connection, &first.sandbox_id);
    port.connect(
        "1",
        &first.sandbox_id,
        ConnectRequest { timeout: Some(600) },
    )
    .unwrap();
    assert_eq!(input_tail(&connection), tail);
    assert!(deadline(&connection, &first.sandbox_id) > old_deadline);
    for (kind, route) in DATA_KINDS.into_iter().zip(&routes) {
        assert!(!*route.revocation.borrow());
        assert_eq!(
            port.application()
                .data_permit(&first.envd_access_token, kind)
                .unwrap()
                .target,
            route.target
        );
    }
    drop(port);
    let path = directory.path().join("controller.db");
    let mut journal = complete_fixture::open(&path, NAMESPACE).unwrap();
    let world = journal.world().unwrap();
    // Use the old deadline as the exact timer hint, without waiting for time
    // to pass. SQL lifetime authority must defeat a still-Running warm cache.
    assert!(journal.due_lifetimes(old_deadline).unwrap().is_empty());
    assert!(matches!(
        journal.accept_expiry(
            core::SandboxId(1),
            core::OperationId(2),
            old_deadline,
            &mut NeverCrash,
        ),
        Err(AdmissionError::NotDue)
    ));
    assert_eq!(journal.world().unwrap(), world);
    assert_eq!(input_tail(&connection), tail);
    let port = new_port(
        journal,
        restore_host(std::slice::from_ref(&first), Arc::new(AtomicUsize::new(0))),
        config(),
    );
    for kind in DATA_KINDS {
        assert!(
            port.application()
                .data_permit(&first.envd_access_token, kind)
                .is_ok()
        );
    }
    port.kill("1", &first.sandbox_id).unwrap();
}

struct FailAt(CrashPoint);

impl CrashInjector for FailAt {
    fn hit(&mut self, point: CrashPoint) -> Result<(), JournalError> {
        if point == self.0 {
            Err(JournalError::InjectedCrash(point))
        } else {
            Ok(())
        }
    }
}

#[test]
fn injected_kill_error_preserves_the_same_journals_committed_authority() {
    for point in [CrashPoint::BeforeCommit, CrashPoint::AfterCommit] {
        let (directory, port) = setup();
        let first = port.create("1", request()).unwrap();
        drop(port);
        let path = directory.path().join("controller.db");
        let mut journal = complete_fixture::open(&path, NAMESPACE).unwrap();
        assert_eq!(
            journal
                .world()
                .unwrap()
                .sandbox_record(core::SandboxId(1))
                .unwrap()
                .phase,
            core::Phase::Running
        );
        assert!(matches!(
            journal.apply(
                core::ControllerInput::Request(core::Request::Kill(core::KillRequest {
                    actor: core::Actor::Tenant(core::TenantId(1)),
                    sandbox: core::SandboxId(1),
                    operation: core::OperationId(2),
                })),
                &mut FailAt(point),
            ),
            Err(JournalError::InjectedCrash(actual)) if actual == point
        ));
        let stops = Arc::new(AtomicUsize::new(0));
        // Deliberately keep the same journal object after the injected error.
        let port = new_port(
            journal,
            restore_host(std::slice::from_ref(&first), Arc::clone(&stops)),
            config(),
        );
        for kind in DATA_KINDS {
            let result = port
                .application()
                .data_permit(&first.envd_access_token, kind);
            if point == CrashPoint::AfterCommit {
                assert_eq!(result.unwrap_err(), PortError::NotFound);
            } else {
                assert!(!*result.unwrap().revocation.borrow());
            }
        }
        assert_eq!(stops.load(Ordering::SeqCst), 0);
        port.kill("1", &first.sandbox_id).unwrap();
        assert_eq!(stops.load(Ordering::SeqCst), 1);
    }
}

#[test]
fn warmed_data_permit_contending_with_kill_cannot_reuse_running_authority() {
    let gate = Arc::new(ActionGate::new());
    let stops = Arc::new(AtomicUsize::new(0));
    let (_directory, port) = setup_with_host(FakeHost {
        stop_gate: Some(Arc::clone(&gate)),
        stop_calls: Some(Arc::clone(&stops)),
        ..FakeHost::default()
    });
    let first = port.create("1", request()).unwrap();
    let routes = DATA_KINDS.map(|kind| {
        port.application()
            .data_permit(&first.envd_access_token, kind)
            .unwrap()
    });
    let killing = Arc::clone(&port);
    let external = first.sandbox_id.clone();
    let worker = std::thread::spawn(move || killing.kill("1", &external));
    gate.entered.wait(); // Kill is committed; its exact host stop holds the lock.
    assert!(routes.iter().all(|route| *route.revocation.borrow()));
    let ready = Arc::new(Barrier::new(2));
    let caller_ready = Arc::clone(&ready);
    let authorizing = Arc::clone(&port);
    let caller = std::thread::spawn(move || {
        caller_ready.wait();
        authorizing
            .application()
            .data_permit(&first.envd_access_token, DataKind::Exec)
    });
    ready.wait();
    gate.release.wait();
    assert_eq!(worker.join().unwrap(), Ok(()));
    assert_eq!(caller.join().unwrap().unwrap_err(), PortError::NotFound);
    assert_eq!(stops.load(Ordering::SeqCst), 1);
    assert!(routes.iter().all(|route| *route.revocation.borrow()));
}
