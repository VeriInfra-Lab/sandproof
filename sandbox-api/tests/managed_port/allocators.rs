//! Public refusal/unknown semantics around durable identifier exhaustion.
use super::*;

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
        complete_fixture::open(directory.path().join("controller.db"), NAMESPACE).unwrap();
    let port = new_port(journal, host, config());
    assert!(matches!(
        port.create("1", request()),
        Err(PortError::QuotaExceeded)
    ));
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    drop(port);
    let journal =
        complete_fixture::open(directory.path().join("controller.db"), NAMESPACE).unwrap();
    assert!(journal.world().unwrap().sandbox_records().is_empty());
    assert_eq!(journal.next_sandbox_id().unwrap(), core::SandboxId(1));
}

#[test]
fn insufficient_future_receipt_headroom_refuses_before_acceptance() {
    let (directory, port) = setup();
    drop(port);
    // One receipt fits, but the complete opener reserves future workflow and
    // cleanup receipts before admitting anything. This is a refusal, not an
    // accepted operation with an unknown result.
    seed_receipt_frontier(&directory, i64::MAX - 1);
    let calls = Arc::new(AtomicUsize::new(0));
    let host = FakeHost {
        prepare_calls: Some(Arc::clone(&calls)),
        ..FakeHost::default()
    };
    let journal =
        complete_fixture::open(directory.path().join("controller.db"), NAMESPACE).unwrap();
    let port = new_port(journal, host, config());
    assert!(matches!(
        port.create("1", request()),
        Err(PortError::QuotaExceeded)
    ));
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    drop(port);
    let journal =
        complete_fixture::open(directory.path().join("controller.db"), NAMESPACE).unwrap();
    assert_eq!(journal.next_sandbox_id().unwrap(), core::SandboxId(1));
    let world = journal.world().unwrap();
    assert!(world.sandbox_records().is_empty());
    assert!(journal.uncertain_actions().unwrap().is_empty());
}
