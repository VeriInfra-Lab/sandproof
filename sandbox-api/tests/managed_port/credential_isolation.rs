//! The actual managed controller and `SQLite` index, with only host I/O faulted.
use super::*;
use sandproof_api::SandboxSession;
use sandproof_controller_service::application::DataKind;
use sha2::{Digest, Sha256};

fn live_pair() -> (TempDir, SandboxSession, SandboxSession) {
    let (directory, port) = setup();
    let first = port.create("1", request()).unwrap();
    let second = port.create("2", request()).unwrap();
    drop(port);
    (directory, first, second)
}

fn saved_host(first: &SandboxSession, second: &SandboxSession) -> FakeHost {
    FakeHost {
        secrets: BTreeMap::from([
            (1, (first.envd_access_token.clone(), BTreeMap::new())),
            (2, (second.envd_access_token.clone(), BTreeMap::new())),
        ])
        .into(),
        ..FakeHost::default()
    }
}

#[test]
fn one_host_credential_fault_does_not_block_the_other_sandbox_after_restart() {
    for fault in ["unreadable", "missing", "changed"] {
        let (directory, first, second) = live_pair();
        let mut host = saved_host(&first, &second);
        match fault {
            "unreadable" => host.token_error_for = Some(core::SandboxId(1)),
            "missing" => {
                host.secrets.lock().unwrap().remove(&1);
            }
            "changed" => host.secrets.lock().unwrap().get_mut(&1).unwrap().0 = "f".repeat(64),
            _ => unreachable!(),
        }
        let port = new_port(
            complete_fixture::open(directory.path().join("controller.db"), NAMESPACE).unwrap(),
            host,
            config(),
        );
        port.application().recover_on_startup().unwrap();
        for kind in [DataKind::Exec, DataKind::FileRead, DataKind::FileWrite] {
            assert_eq!(
                port.application()
                    .authorize_data(&first.envd_access_token, kind),
                Err(PortError::Busy)
            );
            let permit = port
                .application()
                .data_permit(&second.envd_access_token, kind)
                .unwrap();
            assert_eq!(permit.target.vm, core::VmId(1));
            assert!(!*permit.revocation.borrow());
        }
        assert_eq!(
            port.get("2", &second.sandbox_id).unwrap().envd_access_token,
            second.envd_access_token
        );
        assert_eq!(port.get("1", &second.sandbox_id), Err(PortError::NotFound));
        assert_eq!(port.get("1", &first.sandbox_id), Err(PortError::Busy));
        assert_eq!(
            port.application()
                .authorize_data(&"0".repeat(64), DataKind::Exec),
            Err(PortError::NotFound)
        );
        port.kill("1", &first.sandbox_id).unwrap();
        port.kill("2", &second.sandbox_id).unwrap();
    }
}

#[test]
fn starting_credential_fault_is_local_and_never_rotates_or_repairs_a_claimed_identity() {
    for fault in ["changed", "missing_after_claim"] {
        let (directory, journal) = two_unknown_prepares();
        let path = directory.path().join("controller.db");
        let first = core::SandboxId(1);
        let original_registration = journal.data_credential(first).unwrap();
        let original_action = journal
            .uncertain_actions()
            .unwrap()
            .into_iter()
            .find(|action| action.command.command().sandbox() == first)
            .unwrap();
        drop(journal);
        if fault == "missing_after_claim" {
            let database = rusqlite::Connection::open(&path).unwrap();
            assert_eq!(
                database
                    .execute(
                        "DELETE FROM controller_data_credentials WHERE sandbox_id=?1",
                        [format!("sb-{NAMESPACE}-1")]
                    )
                    .unwrap(),
                1
            );
        }
        let prepare_calls = Arc::new(AtomicUsize::new(0));
        let first_token = if fault == "changed" {
            "f".repeat(64)
        } else {
            format!("{:064x}", 1)
        };
        let healthy_token = format!("{:064x}", 2);
        let host = FakeHost {
            secrets: BTreeMap::from([
                (1, (first_token, BTreeMap::new())),
                (2, (healthy_token.clone(), BTreeMap::new())),
            ])
            .into(),
            prepare: PrepareObservation::Successful.into(),
            prepare_calls: Some(Arc::clone(&prepare_calls)),
            ..FakeHost::default()
        };
        let port = new_port(
            complete_fixture::open(&path, NAMESPACE).unwrap(),
            host,
            config(),
        );
        let report = port.application().recover_on_startup().unwrap();
        assert_eq!(report.receipts_recovered, 1);
        assert_eq!(report.workflows_completed, 1);
        assert_eq!(report.unresolved_sandboxes, 1);
        assert_eq!(report.failed_sandboxes, 1);
        assert_eq!(prepare_calls.load(Ordering::SeqCst), 0);
        for kind in [DataKind::Exec, DataKind::FileRead, DataKind::FileWrite] {
            assert_eq!(
                port.application()
                    .authorize_data(&healthy_token, kind)
                    .unwrap()
                    .vm,
                core::VmId(1)
            );
        }
        assert_eq!(
            port.get("2", &format!("sb-{NAMESPACE}-2"))
                .unwrap()
                .envd_access_token,
            healthy_token
        );
        assert_eq!(
            port.get("1", &format!("sb-{NAMESPACE}-1")),
            Err(PortError::NotFound)
        );
        drop(port);
        let journal = complete_fixture::open(&path, NAMESPACE).unwrap();
        assert_eq!(
            journal
                .world()
                .unwrap()
                .sandbox_record(first)
                .unwrap()
                .phase,
            core::Phase::Starting
        );
        assert_eq!(
            journal.data_credential(first).unwrap(),
            if fault == "changed" {
                original_registration
            } else {
                None
            }
        );
        assert_eq!(journal.uncertain_actions().unwrap(), vec![original_action]);
        // Both local target faults leave the shared index healthy, never poisoned.
        assert_eq!(
            journal
                .lookup_data_credential(&Sha256::digest(healthy_token.as_bytes()).into())
                .unwrap(),
            Some((core::SandboxId(2), 1))
        );
    }
}

#[test]
fn missing_shared_registration_denies_data_instead_of_scanning_or_repairing_hosts() {
    let (directory, first, second) = live_pair();
    let path = directory.path().join("controller.db");
    let port = new_port(
        complete_fixture::open(&path, NAMESPACE).unwrap(),
        saved_host(&first, &second),
        config(),
    );
    let database = rusqlite::Connection::open(&path).unwrap();
    assert_eq!(
        database
            .execute(
                "DELETE FROM controller_data_credentials WHERE sandbox_id=?1",
                [&first.sandbox_id]
            )
            .unwrap(),
        1
    );
    drop(database);
    assert_eq!(
        port.application()
            .authorize_data(&second.envd_access_token, DataKind::Exec),
        Err(PortError::Busy)
    );
    // An external fault invalidates the original process's trusted cache.
    // Restart validates the actual image; it must not rebuild Running tokens.
    assert_eq!(
        port.application().recover_on_startup(),
        Err(PortError::Busy)
    );
    drop(port);
    let port = new_port(
        complete_fixture::open(&path, NAMESPACE).unwrap(),
        saved_host(&first, &second),
        config(),
    );
    port.application().recover_on_startup().unwrap();
    assert_eq!(
        port.application()
            .authorize_data(&second.envd_access_token, DataKind::FileRead),
        Err(PortError::Busy)
    );
    port.kill("1", &first.sandbox_id).unwrap();
    port.kill("2", &second.sandbox_id).unwrap();
}

#[test]
fn shared_poison_denies_new_and_previously_issued_relay_permits_but_allows_cleanup() {
    let (directory, first, second) = live_pair();
    let path = directory.path().join("controller.db");
    let calls = Arc::new(AtomicUsize::new(0));
    let host = FakeHost {
        relay_calls: Some(Arc::clone(&calls)),
        ..saved_host(&first, &second)
    };
    let port = new_port(
        complete_fixture::open(&path, NAMESPACE).unwrap(),
        host,
        config(),
    );
    let permit = port
        .application()
        .data_permit(&second.envd_access_token, DataKind::Exec)
        .unwrap();
    let database = rusqlite::Connection::open(&path).unwrap();
    assert_eq!(
        database
            .execute(
                "UPDATE controller_data_credential_state SET poisoned=1 WHERE id=1",
                []
            )
            .unwrap(),
        1
    );
    drop(database);
    for session in [&first, &second] {
        assert_eq!(
            port.application()
                .authorize_data(&session.envd_access_token, DataKind::Exec),
            Err(PortError::Busy)
        );
    }
    assert!(matches!(
        port.application().spawn_data_relay(&permit, false),
        Err(PortError::Busy)
    ));
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    drop(permit);
    drop(port);
    // Valid durable poison survives restart without preventing the operator's
    // exact lifecycle cleanup. Corrupt/unmanaged writes are never trusted by
    // an already-running process just to keep this test green.
    let port = new_port(
        complete_fixture::open(&path, NAMESPACE).unwrap(),
        saved_host(&first, &second),
        config(),
    );
    port.kill("1", &first.sandbox_id).unwrap();
    port.kill("2", &second.sandbox_id).unwrap();
    assert_eq!(port.application().maintain().unwrap().retired_sandboxes, 2);
    drop(port);
    let port = new_port(
        complete_fixture::open(&path, NAMESPACE).unwrap(),
        FakeHost::default(),
        config(),
    );
    assert_eq!(
        port.application()
            .authorize_data(&second.envd_access_token, DataKind::Exec),
        Err(PortError::Busy)
    );
    assert!(
        port.list("1", ListQuery::default())
            .unwrap()
            .items
            .is_empty()
    );
}
