//! Deterministic real-lock regressions, without fork scheduling or sleeps.
use super::*;

const NAMESPACE: &str = "0123456789abcdef";

fn inherited_reference(lease: &ControllerLease) -> File {
    // dup and a fork-before-exec reference retain the same open description.
    lease.file.try_clone().unwrap()
}

fn assert_second_writer_rejected(path: &Path) {
    assert!(matches!(
        ControllerJournal::open(path, NAMESPACE),
        Err(JournalError::ControllerBusy)
    ));
}

#[test]
fn live_controller_keeps_its_exclusive_writer_lease() {
    let scratch = tempfile::tempdir().unwrap();
    let path = scratch.path().join("controller.db");
    let journal = ControllerJournal::open(&path, NAMESPACE).unwrap();
    assert_second_writer_rejected(&path);
    assert_eq!(journal.next_sandbox_id().unwrap(), core::SandboxId(1));
    assert!(journal.world().unwrap().sandbox_records().is_empty());
}

#[test]
fn journal_drop_releases_lease_while_an_inherited_reference_remains() {
    let scratch = tempfile::tempdir().unwrap();
    let path = scratch.path().join("controller.db");
    let journal = ControllerJournal::open(&path, NAMESPACE).unwrap();
    let ControllerJournal { _lease: lease, .. } = &journal;
    let inherited = inherited_reference(lease);
    assert_second_writer_rejected(&path);
    drop(journal);

    // The unrelated reference must not remain an owner of the controller lock.
    let reopened = ControllerJournal::open(&path, NAMESPACE)
        .expect("closing the controller must release its lease before exec");
    assert_second_writer_rejected(&path);
    drop(inherited);
    // Closing an old reference must not release the new controller's lock.
    assert_second_writer_rejected(&path);
    assert_eq!(reopened.next_sandbox_id().unwrap(), core::SandboxId(1));
}

#[test]
fn failed_open_releases_local_lease_while_an_inherited_reference_remains() {
    let scratch = tempfile::tempdir().unwrap();
    let path = scratch.path().join("controller.db");
    // A real unsupported-format rejection at the same opener used by the
    // formal service. This does not enable historical stores in that service.
    drop(Journal::open(&path).unwrap());
    let before = std::fs::read(&path).unwrap();
    let mut inherited = None;
    let failed = (|| -> Result<(), JournalError> {
        let (path, lease) = lease_controller_store(&path)?;
        inherited = Some(inherited_reference(&lease));
        // Failure occurs before a ControllerJournal can own the local lease.
        let _journal =
            Journal::open_controller(&path, true, true, StoreVersionPolicy::CurrentOnly)?;
        Ok(())
    })();
    assert!(matches!(failed, Err(JournalError::UnsupportedStore)));
    assert_eq!(std::fs::read(&path).unwrap(), before);
    assert!(inherited.is_some());

    let reopened = ControllerJournal::open(&path, NAMESPACE)
        .expect("an opener error must release its lease before exec");
    assert_second_writer_rejected(&path);
    drop(inherited);
    assert_second_writer_rejected(&path);
    assert!(reopened.world().unwrap().sandbox_records().is_empty());
}
