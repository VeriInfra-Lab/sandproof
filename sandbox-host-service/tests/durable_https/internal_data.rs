//! Full binding authorization on the original application/SQLite, not a fake
//! permit implementation. Controlled host effects remain component evidence.
use super::{Arc, AtomicI64, Mutex, fixture, local};
use sandproof_controller_service::application::{ApplicationError, DataKind};
use sandproof_host_service::HostLifecycle;
use sandproof_internal::LifecycleReceiver;
use sandproof_protocol::internal::lifecycle::{Intent, LifecycleFact};
use std::sync::atomic::Ordering;

#[tokio::test]
async fn original_internal_data_permit_rejects_every_foreign_identity_and_unadmitted_request() {
    let dir = tempfile::tempdir().unwrap();
    let state = Arc::new(Mutex::new(local::State::default()));
    let app = local::application(
        &dir.path().join("db"),
        Arc::clone(&state),
        Arc::new(local::TestClock(AtomicI64::new(10))),
        true,
    );
    let host = HostLifecycle::new(Arc::clone(&app));
    let create = local::request();
    host.receive(create.clone()).await.unwrap();
    assert!(matches!(
        app.internal_data_permit(&create.identity, DataKind::Exec),
        Err(ApplicationError::NotFound)
    ));
    host.advance(create.clone(), Some(local::template()), local::compute())
        .await
        .unwrap();
    for kind in [DataKind::Exec, DataKind::FileRead, DataKind::FileWrite] {
        let permit = app.internal_data_permit(&create.identity, kind).unwrap();
        assert!(!*permit.revocation.borrow());
    }
    for variant in 0..9 {
        let mut bad = create.identity.clone();
        match variant {
            0 => bad.protocol_version = 2,
            1 => bad.node.deployment_id = "other-deployment".try_into().unwrap(),
            2 => bad.node.node_id = "other-host".try_into().unwrap(),
            3 => bad.node.node_generation = 2.try_into().unwrap(),
            4 => bad.tenant_id = "other-tenant".try_into().unwrap(),
            5 => bad.sandbox_id = "other-sandbox".try_into().unwrap(),
            6 => bad.operation_id = "other-operation".try_into().unwrap(),
            7 => bad.intent_version = 2.try_into().unwrap(),
            _ => bad.request_window = 2.try_into().unwrap(),
        }
        assert!(
            app.internal_data_permit(&bad, DataKind::Exec).is_err(),
            "variant {variant}"
        );
    }
    assert!(
        app.internal_data_permit(&create.identity, DataKind::Exec)
            .is_ok()
    );
}

#[tokio::test]
async fn current_renewed_binding_uses_original_native_eligibility_and_destroy_revokes_permits() {
    let dir = tempfile::tempdir().unwrap();
    let state = Arc::new(Mutex::new(local::State::default()));
    let clock = Arc::new(local::TestClock(AtomicI64::new(10)));
    let app = local::application(&dir.path().join("db"), state, Arc::clone(&clock), true);
    let host = HostLifecycle::new(Arc::clone(&app));
    let create = local::request();
    host.receive(create.clone()).await.unwrap();
    host.advance(create.clone(), Some(local::template()), local::compute())
        .await
        .unwrap();
    let mut renewal = create.clone();
    renewal.intent = Intent::Renew { end_at_ms: 90_010 };
    renewal.identity.operation_id = "operation-2".try_into().unwrap();
    renewal.identity.intent_version = 2.try_into().unwrap();
    clock.0.store(100, Ordering::SeqCst);
    host.receive(renewal.clone()).await.unwrap();
    assert!(matches!(
        app.internal_data_permit(&create.identity, DataKind::Exec),
        Err(ApplicationError::NotFound)
    ));
    assert_eq!(
        host.advance(renewal.clone(), None, local::compute())
            .await
            .unwrap()
            .fact,
        LifecycleFact::Renewed { end_at_ms: 90_010 }
    );
    let permit = app
        .internal_data_permit(&renewal.identity, DataKind::Exec)
        .unwrap();
    let mut destroy = create.clone();
    destroy.intent = Intent::Destroy;
    destroy.identity.operation_id = "operation-3".try_into().unwrap();
    destroy.identity.intent_version = 3.try_into().unwrap();
    host.receive(destroy.clone()).await.unwrap();
    assert!(*permit.revocation.borrow());
    for identity in [&create.identity, &renewal.identity, &destroy.identity] {
        assert!(matches!(
            app.internal_data_permit(identity, DataKind::Exec),
            Err(ApplicationError::NotFound)
        ));
    }
    assert!(matches!(
        app.spawn_data_relay(&permit, false),
        Err(ApplicationError::NotFound)
    ));
    drop(permit);
    assert_eq!(
        host.advance(destroy, None, local::compute())
            .await
            .unwrap()
            .fact,
        LifecycleFact::Cleaned
    );
}

#[tokio::test]
async fn native_expiry_refuses_new_internal_permit_and_original_relay_launch_before_scan() {
    let dir = tempfile::tempdir().unwrap();
    let state = Arc::new(Mutex::new(local::State::default()));
    let clock = Arc::new(local::TestClock(AtomicI64::new(10)));
    let app = local::application(&dir.path().join("db"), state, Arc::clone(&clock), true);
    let host = HostLifecycle::new(Arc::clone(&app));
    let create = local::request();
    host.receive(create.clone()).await.unwrap();
    host.advance(create.clone(), Some(local::template()), local::compute())
        .await
        .unwrap();
    let permit = app
        .internal_data_permit(&create.identity, DataKind::Exec)
        .unwrap();
    clock.0.store(60_010, Ordering::SeqCst);
    assert!(matches!(
        app.internal_data_permit(&create.identity, DataKind::Exec),
        Err(ApplicationError::NotFound)
    ));
    assert!(matches!(
        app.spawn_data_relay(&permit, false),
        Err(ApplicationError::NotFound)
    ));
    drop(permit);
    let (_, expiry) = host.recover().await.unwrap();
    assert_eq!(expiry.completed, 1);
}

#[tokio::test]
async fn one_broken_credential_or_launch_config_cannot_block_another_internal_binding() {
    let dir = tempfile::tempdir().unwrap();
    let state = Arc::new(Mutex::new(local::State::default()));
    let app = local::application(
        &dir.path().join("db"),
        Arc::clone(&state),
        Arc::new(local::TestClock(AtomicI64::new(10))),
        true,
    );
    let host = HostLifecycle::new(Arc::clone(&app));
    let first = local::request();
    let mut other = first.clone();
    other.identity.sandbox_id = "sandbox-2".try_into().unwrap();
    other.identity.operation_id = "operation-2".try_into().unwrap();
    other.identity.tenant_id = "2".try_into().unwrap();
    for create in [&first, &other] {
        host.receive(create.clone()).await.unwrap();
        host.advance(create.clone(), Some(local::template()), local::compute())
            .await
            .unwrap();
    }
    let original = state
        .lock()
        .unwrap()
        .configurations
        .get(&1)
        .unwrap()
        .clone();
    state.lock().unwrap().configurations.get_mut(&1).unwrap().0 = "0".repeat(64);
    assert!(matches!(
        app.internal_data_permit(&first.identity, DataKind::Exec),
        Err(ApplicationError::Busy)
    ));
    assert!(
        app.internal_data_permit(&other.identity, DataKind::Exec)
            .is_ok()
    );
    state.lock().unwrap().configurations.insert(1, original);
    state
        .lock()
        .unwrap()
        .configurations
        .get_mut(&1)
        .unwrap()
        .1
        .metadata
        .insert("changed".into(), "not-original".into());
    assert!(
        app.internal_data_permit(&first.identity, DataKind::Exec)
            .is_err()
    );
    assert!(
        app.internal_data_permit(&other.identity, DataKind::Exec)
            .is_ok()
    );
    state.lock().unwrap().configurations.remove(&1);
    assert!(matches!(
        app.internal_data_permit(&first.identity, DataKind::Exec),
        Err(ApplicationError::Busy)
    ));
    assert!(
        app.internal_data_permit(&other.identity, DataKind::Exec)
            .is_ok()
    );
    assert!(
        app.internal_pending(&fixture::node("host-a"))
            .unwrap()
            .is_empty()
    );
}
