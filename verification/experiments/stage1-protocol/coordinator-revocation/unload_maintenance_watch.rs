//! Exercise the public coordinator, two live watch receivers and the journal's
//! existing housekeeping fault. Host observations come from the component fixture.
use super::super::HOUSEKEEPING_FAILURE;
use crate::application::{ControllerApplication, DataKind, DataPermit};
use crate::controller::tests_support::lifecycle_application as lifecycle;
use crate::{ControllerJournal, JournalError, NeverCrash};
use lifecycle::fixture::{State, TestEpoch, TestHost};
use sandproof_protocol::lifecycle::{
    AcceptedLifecycleIntent, ExpectedSession, LifecycleAccessIdentity, LifecycleOperation,
};
use std::path::Path;
use std::sync::{Arc, Mutex};

fn start_sessions(app: &ControllerApplication<TestHost>) -> [AcceptedLifecycleIntent; 2] {
    let target = lifecycle::request();
    let mut peer = target.clone();
    peer.request.binding.session = "session-b".try_into().unwrap();
    peer.retention.slot = 2;
    peer.request.binding.key.owner = "account-beta".try_into().unwrap();
    let creates = [target, peer];
    let context = lifecycle::context();
    for create in &creates {
        app.record_lifecycle_intent(create, &context).unwrap();
    }
    app.establish_lifecycle_host_epoch().unwrap();
    let sandboxes = creates.each_ref().map(|create| {
        app.advance_lifecycle_create_with_verification(
            create,
            &context,
            Some(lifecycle::fixture::template()),
            lifecycle::fixture::compute(),
            None,
        )
        .unwrap()
        .local
        .unwrap()
        .sandbox
    });
    assert_ne!(sandboxes[0], sandboxes[1]);
    creates
}

fn open_route(
    app: &ControllerApplication<TestHost>,
    create: &AcceptedLifecycleIntent,
) -> DataPermit {
    let binding = &create.request.binding;
    let LifecycleOperation::Create {
        target_incarnation, ..
    } = create.request.operation
    else {
        panic!("expected the running Create")
    };
    let access = LifecycleAccessIdentity {
        protocol_version: binding.protocol_version,
        center: create.center.clone(),
        node: binding.node.clone(),
        session: binding.session.clone(),
        operation: binding.key.clone(),
        accepted_version: create.accepted_version,
        incarnation: target_incarnation,
    };
    let (permit, _) = app
        .lifecycle_data_authorization(&access, &lifecycle::context(), DataKind::FileRead)
        .unwrap();
    assert!(!*permit.revocation.borrow());
    permit
}

fn next_unload(create: &AcceptedLifecycleIntent) -> AcceptedLifecycleIntent {
    let LifecycleOperation::Create {
        target_incarnation, ..
    } = create.request.operation
    else {
        panic!("expected the source incarnation")
    };
    let mut unload = create.clone();
    unload.accepted_version = 2;
    unload.request.binding.expected = ExpectedSession::Version(create.accepted_version);
    unload.request.binding.key.sequence = 2.try_into().unwrap();
    unload.request.operation = LifecycleOperation::Unload {
        source_incarnation: target_incarnation,
        capture: 1.try_into().unwrap(),
    };
    unload
}

fn stored_inbox(journal: &ControllerJournal) -> (Vec<u8>, Vec<u8>, i64) {
    journal
        .journal
        .connection
        .query_row(
            "SELECT state_json,sha256,reserve_bytes FROM controller_lifecycle_inbox WHERE id=1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .unwrap()
}

#[path = "unload_checkpoint_publication.rs"]
mod publication;

fn record_with_fault(
    app: &ControllerApplication<TestHost>,
    creates: &[AcceptedLifecycleIntent; 2],
    unload: &AcceptedLifecycleIntent,
    durable: bool,
) {
    let [target, peer] = creates.each_ref().map(|create| open_route(app, create));
    HOUSEKEEPING_FAILURE.with(|slot| assert!(slot.replace(Some(durable)).is_none()));
    assert!(matches!(
        app.record_lifecycle_intent(unload, &lifecycle::context()),
        Err(JournalError::CommitUncertain(rusqlite::Error::InvalidQuery))
    ));
    HOUSEKEEPING_FAILURE.with(|slot| assert!(slot.get().is_none()));
    assert!(*target.revocation.borrow());
    assert!(*peer.revocation.borrow());
}

fn assert_unrecorded(
    journal: &ControllerJournal,
    creates: &[AcceptedLifecycleIntent; 2],
    unload: &AcceptedLifecycleIntent,
) {
    let context = lifecycle::context();
    assert!(matches!(
        journal.lifecycle_work_state(unload, &context),
        Err(JournalError::Conflict)
    ));
    for create in creates {
        assert!(
            journal
                .lifecycle_work_state(create, &context)
                .unwrap()
                .current
        );
    }
}

fn retry_unload(
    app: &ControllerApplication<TestHost>,
    creates: &[AcceptedLifecycleIntent; 2],
    unload: &AcceptedLifecycleIntent,
) {
    let context = lifecycle::context();
    let [target, peer] = creates.each_ref().map(|create| open_route(app, create));
    let accepted = app.record_lifecycle_intent(unload, &context).unwrap();
    assert_eq!(&accepted.intent, unload);
    assert!(accepted.current);
    assert!(*target.revocation.borrow());
    assert!(!*peer.revocation.borrow());
    assert_eq!(
        app.lifecycle_work_state(unload, &context).unwrap(),
        accepted
    );
    assert!(
        app.lifecycle_work_state(&creates[1], &context)
            .unwrap()
            .current
    );
}

fn check_maintenance_failure(path: &Path, durable: bool) {
    let host = Arc::new(Mutex::new(State {
        epoch: TestEpoch::Matching,
        ..State::default()
    }));
    let app = lifecycle::application(path, host.clone(), true);
    let creates = start_sessions(&app);
    let unload = next_unload(&creates[0]);
    drop(app);

    // Leave a published, covered prefix for the next writer. Opening the
    // application and acquiring data permits use only reads of this state.
    let mut journal = lifecycle::open(path);
    journal.publish_checkpoint(&mut NeverCrash).unwrap();
    let before = journal.checkpoint_progress().unwrap().unwrap();
    assert!(before.deleted_through < before.input_cut);
    let world = journal.world().unwrap();
    let inbox = stored_inbox(&journal);
    drop(journal);

    let app = lifecycle::application(path, host.clone(), false);
    record_with_fault(&app, &creates, &unload, durable);
    drop(app);

    // Before retry, check the outcome on a reopened store. Only the covered
    // prefix may move. Neither the input high-water mark nor the inbox advanced.
    let journal = lifecycle::open(path);
    let after = journal.checkpoint_progress().unwrap().unwrap();
    let batch = lifecycle::maintenance().prune_batch_rows;
    let deleted = if durable {
        batch.min(before.input_cut - before.deleted_through)
    } else {
        0
    };
    assert_eq!(after.deleted_through, before.deleted_through + deleted);
    assert_eq!(after.input_cut, before.input_cut);
    assert_eq!(after.input_high_water, before.input_high_water);
    assert_eq!(journal.world().unwrap(), world);
    assert_eq!(stored_inbox(&journal), inbox);
    assert_unrecorded(&journal, &creates, &unload);
    drop(journal);

    // Retrying the same Unload after recovery targets only its own sandbox.
    let app = lifecycle::application(path, host, false);
    retry_unload(&app, &creates, &unload);
}

#[test]
fn maintenance_commit_uncertainty_revokes_both_routes_before_unload_is_recorded() {
    for durable in [false, true] {
        let temp = tempfile::tempdir().unwrap();
        check_maintenance_failure(&temp.path().join("db"), durable);
    }
}
