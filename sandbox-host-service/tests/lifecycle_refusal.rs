//! Current host adapter and original application/SQLite. Host I/O is controlled;
//! this does not exercise TLS, center refund, SDK or a production VM.
#[allow(dead_code, reason = "shared original application fixture")]
#[path = "../../controller-service/tests/support/lifecycle_application.rs"]
mod fixture;
use sandproof_host_service::HostLifecycle;
use sandproof_internal::LifecycleReceiver;
use sandproof_protocol::lifecycle::{
    CreateRefusalReason, CurrentCreateReadiness, ExpectedSession, LifecycleOperation,
    LifecycleWindowClose, TerminalStatus,
};
use std::sync::{Arc, Mutex};

#[tokio::test]
async fn original_capacity_refusal_survives_restart_and_projects_through_current_destroy() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("db");
    let state = Arc::new(Mutex::new(fixture::fixture::State {
        epoch: fixture::fixture::TestEpoch::Matching,
        disk_available: Some(0),
        ..fixture::fixture::State::default()
    }));
    let application = Arc::new(fixture::application(&path, state.clone(), true));
    application.establish_lifecycle_host_epoch().unwrap();
    let host = HostLifecycle::new(application);
    let create = fixture::request();
    let context = fixture::context();
    host.record_lifecycle(create.clone(), context.clone())
        .await
        .unwrap();
    let refused = host
        .advance(
            create.clone(),
            Some(fixture::fixture::template()),
            fixture::fixture::compute(),
        )
        .await
        .unwrap();
    assert_eq!(refused.refusal, Some(CreateRefusalReason::Capacity));
    assert!(refused.local.is_none());
    let observed = host
        .observe_create(create.clone(), context.clone())
        .await
        .unwrap();
    assert!(observed.matches_request(&create));
    assert_eq!(
        observed.readiness,
        CurrentCreateReadiness::Refused {
            reason: CreateRefusalReason::Capacity
        }
    );
    assert!(!observed.historical_create_succeeded);
    assert!(
        host.pending(context.expected_node().clone())
            .await
            .unwrap()
            .is_empty()
    );
    drop(host);

    state.lock().unwrap().disk_available = None;
    let application = Arc::new(fixture::application(&path, state.clone(), false));
    let host = HostLifecycle::new(application);
    assert_eq!(
        host.advance(create.clone(), None, fixture::fixture::compute())
            .await
            .unwrap(),
        refused
    );
    assert!(state.lock().unwrap().actions.is_empty());
    let mut destroy = create.clone();
    destroy.accepted_version = 2;
    destroy.request.binding.key.sequence = 2.try_into().unwrap();
    destroy.request.binding.expected = ExpectedSession::Version(1);
    destroy.request.operation = LifecycleOperation::Destroy {};
    host.record_lifecycle(destroy.clone(), context.clone())
        .await
        .unwrap();
    let observed = host
        .observe_create(create.clone(), context.clone())
        .await
        .unwrap();
    assert_eq!(observed.current_request, destroy);
    assert!(observed.matches_request(&create));
    assert_eq!(
        observed.readiness,
        CurrentCreateReadiness::Refused {
            reason: CreateRefusalReason::Capacity
        }
    );
    let terminal = host
        .observe_terminal(destroy, context.clone())
        .await
        .unwrap();
    assert_eq!(terminal.status, TerminalStatus::Cleaned);
    let close = LifecycleWindowClose {
        terminal: terminal.clone(),
    };
    assert!(matches!(
        host.close_window(close.clone(), context.clone()).await,
        Err(sandproof_internal::ReceiveError::Unavailable)
    ));
    assert_eq!(
        host.confirm_terminal(terminal.clone(), context.clone())
            .await
            .unwrap()
            .result,
        terminal
    );
    assert!(
        host.close_window(close.clone(), context.clone())
            .await
            .unwrap()
            .matches(&close)
    );
    assert!(
        host.close_window(close.clone(), context)
            .await
            .unwrap()
            .matches(&close)
    );
    assert!(state.lock().unwrap().actions.is_empty());
}

#[tokio::test]
async fn current_host_receiver_confirms_only_actual_renewal_and_reopens_the_same_checkpoint() {
    use sandproof_protocol::lifecycle::RenewalConfirmation;
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("db");
    let state = Arc::new(Mutex::new(fixture::fixture::State {
        epoch: fixture::fixture::TestEpoch::Matching,
        ..fixture::fixture::State::default()
    }));
    let application = Arc::new(fixture::application(&path, state.clone(), true));
    application.establish_lifecycle_host_epoch().unwrap();
    let host = HostLifecycle::new(Arc::clone(&application));
    let create = fixture::request();
    let context = fixture::context();
    host.record_lifecycle(create.clone(), context.clone())
        .await
        .unwrap();
    host.advance(
        create.clone(),
        Some(fixture::fixture::template()),
        fixture::fixture::compute(),
    )
    .await
    .unwrap();
    let LifecycleOperation::Create {
        target_incarnation, ..
    } = create.request.operation
    else {
        panic!()
    };
    let mut sdk = create.clone();
    sdk.accepted_version = 2;
    sdk.request.binding.expected = ExpectedSession::Version(1);
    sdk.request.binding.key.window = 2.try_into().unwrap();
    sdk.request.binding.key.sequence = 2.try_into().unwrap();
    sdk.request.operation = LifecycleOperation::Renew {
        incarnation: target_incarnation,
        end_at_ms: 90_010,
        extend_only: false,
    };
    let result = RenewalConfirmation {
        request: sdk.clone(),
        effective_end_at_ms: 90_010,
    };
    assert!(matches!(
        host.confirm_renewal(result.clone(), context.clone()).await,
        Err(sandproof_internal::ReceiveError::Conflict)
    ));
    host.record_lifecycle(sdk.clone(), context.clone())
        .await
        .unwrap();
    let mut next = sdk.clone();
    next.accepted_version = 3;
    next.request.binding.expected = ExpectedSession::Version(2);
    next.request.binding.key.sequence = 3.try_into().unwrap();
    assert!(matches!(
        host.record_lifecycle(next.clone(), context.clone()).await,
        Err(sandproof_internal::ReceiveError::Unavailable)
    ));
    let mut changed = result.clone();
    changed.effective_end_at_ms += 1;
    assert!(matches!(
        host.confirm_renewal(changed, context.clone()).await,
        Err(sandproof_internal::ReceiveError::Conflict)
    ));
    assert!(
        host.confirm_renewal(result.clone(), context.clone())
            .await
            .unwrap()
            .matches(&result)
    );
    host.record_lifecycle(next.clone(), context.clone())
        .await
        .unwrap();
    let next_result = RenewalConfirmation {
        request: next,
        effective_end_at_ms: 90_010,
    };
    assert!(
        host.confirm_renewal(next_result.clone(), context.clone())
            .await
            .unwrap()
            .matches(&next_result)
    );
    let actions = state.lock().unwrap().actions.clone();
    drop(host);
    drop(application);
    let application = Arc::new(fixture::application(&path, state.clone(), false));
    let host = HostLifecycle::new(application);
    assert!(
        host.confirm_renewal(next_result.clone(), context.clone())
            .await
            .unwrap()
            .matches(&next_result)
    );
    assert!(matches!(
        host.confirm_renewal(result, context).await,
        Err(sandproof_internal::ReceiveError::Conflict)
    ));
    assert_eq!(
        state.lock().unwrap().actions,
        actions,
        "confirmation and replay must not perform Host effects"
    );
}
