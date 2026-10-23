//! Current full binding on the original application, never a substitute permit.
use super::{Arc, AtomicI64, Mutex, local};
use sandproof_controller_service::application::{ApplicationError, DataKind};
use sandproof_host_service::HostLifecycle;
use sandproof_internal::LifecycleReceiver;
use sandproof_protocol::lifecycle::TerminalStatus;
use std::sync::atomic::Ordering;

#[tokio::test]
async fn original_internal_data_permit_rejects_every_foreign_identity_and_unadmitted_request() {
    let dir = tempfile::tempdir().unwrap(); let state = Arc::new(Mutex::new(local::state()));
    let app = local::application(&dir.path().join("db"), state, Arc::new(local::TestClock(AtomicI64::new(10))), true);
    let host = HostLifecycle::new(app.clone()); let create = local::request(); host.record_lifecycle(create.clone(), local::context()).await.unwrap();
    let identity = local::access(&create);
    assert!(matches!(app.lifecycle_data_authorization(&identity, &local::context(), DataKind::Exec), Err(ApplicationError::NotFound)));
    host.advance(create, Some(local::template()), local::compute()).await.unwrap();
    for kind in [DataKind::Exec, DataKind::FileRead, DataKind::FileWrite] {
        let (permit, _) = app.lifecycle_data_authorization(&identity, &local::context(), kind).unwrap(); assert!(!*permit.revocation.borrow());
    }
    for variant in 0..14 {
        let mut bad = identity.clone();
        match variant {
            0 => bad.protocol_version = 2,
            1 => bad.node.deployment_id = "other-deployment".try_into().unwrap(),
            2 => bad.node.node_id = "other-host".try_into().unwrap(),
            3 => bad.node.node_generation = 2.try_into().unwrap(),
            4 => bad.operation.owner = "other-owner".try_into().unwrap(),
            5 => bad.session = "other-session".try_into().unwrap(),
            6 => bad.operation.sequence = 2.try_into().unwrap(),
            7 => bad.accepted_version = 2,
            8 => bad.operation.window = 2.try_into().unwrap(),
            9 => bad.incarnation = 102.try_into().unwrap(),
            10 => bad.center.center = "other-center".try_into().unwrap(),
            11 => bad.center.deployment = "other-deployment".try_into().unwrap(),
            12 => bad.operation.deployment = "other-deployment".try_into().unwrap(),
            _ => bad.accepted_version = 0,
        }
        assert!(app.lifecycle_data_authorization(&bad, &local::context(), DataKind::Exec).is_err(), "variant {variant}");
    }
    assert!(app.lifecycle_data_authorization(&identity, &local::context(), DataKind::Exec).is_ok());
    let mut wrong_context = local::config(); wrong_context.center.center = "other-center".try_into().unwrap();
    let context = sandproof_protocol::lifecycle::AuthenticatedCenterContext::from_verified_peer_registration(wrong_context.center, wrong_context.node);
    assert!(app.lifecycle_data_authorization(&identity, &context, DataKind::Exec).is_err());
}

#[tokio::test]
async fn current_renewed_binding_uses_original_native_eligibility_and_destroy_revokes_permits() {
    let dir = tempfile::tempdir().unwrap(); let clock = Arc::new(local::TestClock(AtomicI64::new(10)));
    let app = local::application(&dir.path().join("db"), Arc::new(Mutex::new(local::state())), clock.clone(), true);
    let host = HostLifecycle::new(app.clone()); let create = local::request();
    host.record_lifecycle(create.clone(), local::context()).await.unwrap(); host.advance(create.clone(), Some(local::template()), local::compute()).await.unwrap();
    clock.0.store(100, Ordering::SeqCst); let renewal = local::renew(&create, 90_010); host.record_lifecycle(renewal.clone(), local::context()).await.unwrap();
    assert!(matches!(app.lifecycle_data_authorization(&local::access(&create), &local::context(), DataKind::Exec), Err(ApplicationError::NotFound)));
    assert_eq!(app.apply_lifecycle_renew(&renewal, &local::context()).unwrap().1, 90_010);
    let (permit, _) = app.lifecycle_data_authorization(&local::access(&renewal), &local::context(), DataKind::Exec).unwrap();
    let destroy = local::destroy(&renewal); host.record_lifecycle(destroy.clone(), local::context()).await.unwrap();
    assert!(*permit.revocation.borrow());
    for intent in [&create, &renewal, &destroy] {
        assert!(matches!(app.lifecycle_data_authorization(&local::access(intent), &local::context(), DataKind::Exec), Err(ApplicationError::NotFound)));
    }
    assert!(matches!(app.spawn_data_relay(&permit, false), Err(ApplicationError::NotFound)));
    drop(permit); local::drive(&host).await;
    assert_eq!(host.observe_terminal(destroy, local::context()).await.unwrap().status, TerminalStatus::Cleaned);
}

#[tokio::test]
async fn native_expiry_refuses_new_internal_permit_and_original_relay_launch_before_scan() {
    let dir = tempfile::tempdir().unwrap(); let state = Arc::new(Mutex::new(local::state())); let clock = Arc::new(local::TestClock(AtomicI64::new(10)));
    let app = local::application(&dir.path().join("db"), state.clone(), clock.clone(), true); let host = HostLifecycle::new(app.clone()); let create = local::request();
    host.record_lifecycle(create.clone(), local::context()).await.unwrap(); host.advance(create.clone(), Some(local::template()), local::compute()).await.unwrap();
    let (permit, _) = app.lifecycle_data_authorization(&local::access(&create), &local::context(), DataKind::Exec).unwrap();
    clock.0.store(60_010, Ordering::SeqCst);
    assert!(matches!(app.lifecycle_data_authorization(&local::access(&create), &local::context(), DataKind::Exec), Err(ApplicationError::NotFound)));
    assert!(matches!(app.spawn_data_relay(&permit, false), Err(ApplicationError::NotFound)));
    assert!(state.lock().unwrap().released_slots.is_empty());
    drop(permit); host.recover().await.unwrap(); assert_eq!(state.lock().unwrap().released_slots.len(), 1);
}

#[tokio::test]
async fn one_broken_credential_or_launch_config_cannot_block_another_internal_binding() {
    let dir = tempfile::tempdir().unwrap(); let state = Arc::new(Mutex::new(local::state()));
    let app = local::application(&dir.path().join("db"), state.clone(), Arc::new(local::TestClock(AtomicI64::new(10))), true); let host = HostLifecycle::new(app.clone());
    let first = local::request(); let mut other = first.clone();
    other.request.binding.session = "session-2".try_into().unwrap(); other.request.binding.key.sequence = 2.try_into().unwrap(); other.request.binding.key.owner = "2".try_into().unwrap(); other.retention.slot = 2;
    for create in [&first, &other] { host.record_lifecycle(create.clone(), local::context()).await.unwrap(); host.advance(create.clone(), Some(local::template()), local::compute()).await.unwrap(); }
    let first_local = app.lifecycle_work_state(&first, &local::context()).unwrap().local.unwrap().sandbox.0;
    let original = state.lock().unwrap().configurations.get(&first_local).unwrap().clone();
    state.lock().unwrap().configurations.get_mut(&first_local).unwrap().0 = "0".repeat(64);
    assert!(matches!(app.lifecycle_data_authorization(&local::access(&first), &local::context(), DataKind::Exec), Err(ApplicationError::Busy)));
    assert!(app.lifecycle_data_authorization(&local::access(&other), &local::context(), DataKind::Exec).is_ok());
    state.lock().unwrap().configurations.insert(first_local, original);
    state.lock().unwrap().configurations.get_mut(&first_local).unwrap().1.metadata.insert("changed".into(), "not-original".into());
    assert!(app.lifecycle_data_authorization(&local::access(&first), &local::context(), DataKind::Exec).is_err());
    assert!(app.lifecycle_data_authorization(&local::access(&other), &local::context(), DataKind::Exec).is_ok());
    state.lock().unwrap().configurations.remove(&first_local);
    assert!(matches!(app.lifecycle_data_authorization(&local::access(&first), &local::context(), DataKind::Exec), Err(ApplicationError::Busy)));
    assert!(app.lifecycle_data_authorization(&local::access(&other), &local::context(), DataKind::Exec).is_ok());
    assert!(app.lifecycle_pending_creates(&local::config().node).unwrap().is_empty());
}
