//! Public refusal/unknown semantics around durable identifier exhaustion.
use super::*;
use sandproof_controller_service::JournalError;

fn seed_receipt_frontier(directory: &TempDir, value: i64) {
    rusqlite::Connection::open(directory.path().join("controller.db"))
        .unwrap()
        .execute(
            "UPDATE controller_id_highwaters SET high_water=?1 WHERE kind='receipt'",
            [value.to_string()],
        )
        .unwrap();
}

#[test]
fn exhausted_before_admission_is_a_refusal_without_new_identity_or_host_work() {
    let (directory, port) = setup();
    drop(port);
    seed_receipt_frontier(&directory, i64::MAX);
    let calls = Arc::new(AtomicUsize::new(0));
    let host = FakeHost {
        prepare_calls: Some(Arc::clone(&calls)),
        ..FakeHost::default()
    };
    let journal =
        ControllerJournal::open(directory.path().join("controller.db"), NAMESPACE).unwrap();
    let port = ManagedPort::new(journal, host, config());
    assert!(matches!(
        port.create("1", request()),
        Err(PortError::QuotaExceeded)
    ));
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    drop(port);
    let journal =
        ControllerJournal::open(directory.path().join("controller.db"), NAMESPACE).unwrap();
    assert!(journal.world().unwrap().sandbox_records().is_empty());
    assert_eq!(journal.next_sandbox_id().unwrap(), core::SandboxId(1));
}

#[test]
fn exhaustion_after_acceptance_remains_unknown_and_keeps_original_identity_queryable() {
    let (directory, port) = setup();
    drop(port);
    // One receipt still fits at admission. The fence consumes it; a later
    // physical step cannot be recorded. This batch does not reserve an entire
    // workflow's future receipt/cleanup budget at admission.
    seed_receipt_frontier(&directory, i64::MAX - 1);
    let calls = Arc::new(AtomicUsize::new(0));
    let host = FakeHost {
        prepare_calls: Some(Arc::clone(&calls)),
        ..FakeHost::default()
    };
    let journal =
        ControllerJournal::open(directory.path().join("controller.db"), NAMESPACE).unwrap();
    let port = ManagedPort::new(journal, host, config());
    assert!(matches!(
        port.create("1", request()),
        Err(PortError::OutcomeUnknown)
    ));
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    drop(port);
    let mut journal =
        ControllerJournal::open(directory.path().join("controller.db"), NAMESPACE).unwrap();
    assert_eq!(journal.next_sandbox_id().unwrap(), core::SandboxId(2));
    let world = journal.world().unwrap();
    assert_eq!(world.sandbox_records().len(), 1);
    assert_eq!(
        world.sandbox_record(core::SandboxId(1)).unwrap().phase,
        core::Phase::Starting
    );
    let query = core::ControllerInput::Request(core::Request::Query(core::QueryRequest {
        actor: core::Actor::Tenant(core::TenantId(1)),
        sandbox: core::SandboxId(1),
        operation: core::OperationId(1),
    }));
    assert!(journal.check(query).is_ok());
    assert_eq!(journal.uncertain_actions().unwrap().len(), 1);
    let retry = core::ControllerInput::Request(core::Request::Create(core::CreateRequest {
        actor: core::Actor::Tenant(core::TenantId(1)),
        owner: core::TenantId(1),
        sandbox: core::SandboxId(1),
        workspace: core::FsId(1),
        file_policy: core::FilePolicyRef {
            digest: core::PolicyDigest(1),
        },
        policy: core::NetworkPolicy::DenyAll,
        operation: core::OperationId(1),
    }));
    assert!(matches!(
        journal.apply(retry, &mut NeverCrash),
        Err(JournalError::UnresolvedAction)
    ));
    assert_eq!(journal.next_sandbox_id().unwrap(), core::SandboxId(2));
}
