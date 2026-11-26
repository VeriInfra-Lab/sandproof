use super::{State, TestEpoch, advance, application, context, fixture, request};
use sandproof_controller_service::application::branching::{
    BranchAccessIdentity, BranchAccessSelector, BranchBatchIdentity, BranchTarget,
};
use sandproof_controller_service::application::{
    HostBranchBatchKey, HostBranchClose, HostBranchLimits,
};
use sandproof_controller_service::{
    application::{ApplicationError, ControllerApplication, DataAuthorization, DataKind},
    capture::SourceProcessIdentity,
};
use sandproof_protocol::lifecycle::{
    AcceptedLifecycleIntent, ExpectedSession, LifecycleAccessIdentity, LifecycleOperation,
};
use std::{
    path::Path,
    sync::{Arc, Mutex, atomic::AtomicI64},
};

fn running(
    path: &Path,
) -> (
    ControllerApplication<fixture::TestHost>,
    LifecycleAccessIdentity,
    AcceptedLifecycleIntent,
    Arc<Mutex<State>>,
) {
    let state = Arc::new(Mutex::new(State {
        epoch: TestEpoch::Matching,
        source: Some(SourceProcessIdentity {
            pid: 1234,
            start_time_ticks: 5678,
            boot_id: "fixture-boot".into(),
        }),
        ..State::default()
    }));
    let app = application(path, Arc::clone(&state), true)
        .with_clock(Arc::new(fixture::TestClock(AtomicI64::new(10))));
    let create = request();
    let context = context();
    app.record_lifecycle_intent(&create, &context).unwrap();
    app.establish_lifecycle_host_epoch().unwrap();
    advance(&app).unwrap();
    let LifecycleOperation::Create {
        target_incarnation, ..
    } = create.request.operation
    else {
        unreachable!();
    };
    let destroy = create.clone();
    let binding = create.request.binding;
    let parent = LifecycleAccessIdentity {
        protocol_version: binding.protocol_version,
        center: create.center,
        node: binding.node,
        session: binding.session,
        operation: binding.key,
        accepted_version: create.accepted_version,
        incarnation: target_incarnation,
    };
    (app, parent, destroy, state)
}

fn planned_branches() -> [BranchTarget; 2] {
    [
        BranchTarget {
            sandbox: "branch-a".try_into().unwrap(),
            port: 31_000.try_into().unwrap(),
        },
        BranchTarget {
            sandbox: "branch-b".try_into().unwrap(),
            port: 31_001.try_into().unwrap(),
        },
    ]
}

#[test]
fn native_branch_dispatch_requires_live_observation_and_rechecks_revocation() {
    let directory = tempfile::tempdir().unwrap();
    let (app, parent, mut destroy, host_state) = running(&directory.path().join("db"));
    let context = context();
    let mut operation = parent.operation.clone();
    operation.sequence = 9.try_into().unwrap();
    let key = HostBranchBatchKey {
        operation,
        batch: BranchBatchIdentity {
            parent: parent.session.clone(),
            incarnation: parent.incarnation,
            checkpoint: "checkpoint".try_into().unwrap(),
            batch: "batch".try_into().unwrap(),
        },
    };
    let planned = planned_branches();
    app.configure_host_branching(HostBranchLimits {
        max_batches: 2,
        max_branches_per_vm: 2,
    })
    .unwrap();
    app.reserve_host_branch_batch(&parent, &context, &key, &planned)
        .unwrap();
    app.publish_host_branch_batch(&parent, &context, &key, &planned)
        .unwrap();
    let identity = |target: &BranchTarget| BranchAccessIdentity {
        parent: parent.clone(),
        selector: BranchAccessSelector {
            batch: key.batch.clone(),
            operation: key.operation.clone(),
            branch: target.sandbox.clone(),
        },
    };
    let first = identity(&planned[0]);
    let sibling = identity(&planned[1]);
    assert!(matches!(
        app.lifecycle_branch_data_authorization(&first, &context, DataKind::Exec),
        Err(ApplicationError::NotFound)
    ));
    assert_eq!(
        app.observe_host_branch_data_ready(&first, &context, &planned[1]),
        Err(ApplicationError::NotFound)
    );
    app.observe_host_branch_data_ready(&first, &context, &planned[0])
        .unwrap();
    app.observe_host_branch_data_ready(&sibling, &context, &planned[1])
        .unwrap();
    let (mut first, secret) = app
        .lifecycle_branch_data_authorization(&first, &context, DataKind::Exec)
        .unwrap();
    let (mut sibling, sibling_secret) = app
        .lifecycle_branch_data_authorization(&sibling, &context, DataKind::FileRead)
        .unwrap();
    let parent_secret = app
        .lifecycle_data_authorization(&parent, &context, DataKind::Exec)
        .unwrap()
        .1;
    assert_ne!(secret, parent_secret);
    assert_ne!(secret, sibling_secret);
    assert_eq!(
        app.with_host_branch_dispatch(&first, |_, route| Ok(route.port().get())),
        Ok(31_000)
    );

    let close_scope = Some(&planned[0].sandbox);
    app.change_host_branch_close(&parent, &context, &key, HostBranchClose::Begin(close_scope))
        .unwrap();
    assert!(*first.revocation().borrow());
    assert!(!*sibling.revocation().borrow());
    assert_eq!(
        app.with_host_branch_dispatch::<()>(&first, |_, _| panic!("revoked data dispatched")),
        Err(ApplicationError::NotFound)
    );
    assert_eq!(
        app.change_host_branch_close(
            &parent,
            &context,
            &key,
            HostBranchClose::ConfirmGone(close_scope)
        ),
        Err(ApplicationError::Busy)
    );
    drop(first);
    confirm_requires_empty_durable_scope(&app, &parent, &context, &key, close_scope, &host_state);

    destroy.accepted_version = 2;
    destroy.request.binding.expected = ExpectedSession::Version(1);
    destroy.request.binding.key.sequence = 2.try_into().unwrap();
    destroy.request.operation = LifecycleOperation::Destroy {};
    app.record_lifecycle_intent(&destroy, &context).unwrap();
    assert!(*sibling.revocation().borrow());
    assert_eq!(
        app.with_host_branch_dispatch::<()>(&sibling, |_, _| panic!("ended parent dispatched")),
        Err(ApplicationError::NotFound)
    );
}

fn confirm_requires_empty_durable_scope(
    app: &ControllerApplication<fixture::TestHost>,
    parent: &LifecycleAccessIdentity,
    context: &sandproof_protocol::lifecycle::AuthenticatedCenterContext,
    key: &HostBranchBatchKey,
    close_scope: Option<&sandproof_protocol::SandboxId>,
    host_state: &Mutex<State>,
) {
    host_state.lock().unwrap().live_branch_relay = Some(31_000);
    assert_eq!(
        app.change_host_branch_close(
            parent,
            context,
            key,
            HostBranchClose::ConfirmGone(close_scope)
        ),
        Err(ApplicationError::Busy)
    );
    host_state.lock().unwrap().live_branch_relay = None;
    app.change_host_branch_close(
        parent,
        context,
        key,
        HostBranchClose::ConfirmGone(close_scope),
    )
    .unwrap();
}
