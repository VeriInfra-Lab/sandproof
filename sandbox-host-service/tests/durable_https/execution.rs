//! Original application/SQLite + controlled physical effects; not KVM evidence.
use super::{Arc, AtomicI64, Mutex, fixture, local};
use sandproof_host_service::HostLifecycle;
use sandproof_internal::LifecycleReceiver;
use sandproof_protocol::internal::lifecycle::{Intent, LifecycleFact};
use std::sync::atomic::Ordering;

fn destroy(
    create: &sandproof_protocol::internal::lifecycle::LifecycleRequest,
) -> sandproof_protocol::internal::lifecycle::LifecycleRequest {
    let mut r = create.clone();
    r.intent = Intent::Destroy;
    r.identity.operation_id = "operation-2".try_into().unwrap();
    r.identity.intent_version = 2.try_into().unwrap();
    r
}
fn launches(state: &Mutex<local::State>) -> usize {
    state
        .lock()
        .unwrap()
        .actions
        .iter()
        .filter(|a| {
            matches!(
                a.command.command(),
                sandbox_controller_core::Command::PrepareVm { .. }
            )
        })
        .count()
}

#[tokio::test]
async fn original_create_reopen_destroy_and_physical_release_publish_exact_cached_facts() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("db");
    let state = Arc::new(Mutex::new(local::State::default()));
    let clock = Arc::new(local::TestClock(AtomicI64::new(10)));
    let create = local::request();
    let ready;
    {
        let app = local::application(&path, Arc::clone(&state), Arc::clone(&clock), true);
        let service = HostLifecycle::new(app);
        assert_eq!(
            service.receive(create.clone()).await.unwrap().fact,
            LifecycleFact::Recorded
        );
        assert_eq!(launches(&state), 0);
        ready = service
            .advance(create.clone(), Some(local::template()), local::compute())
            .await
            .unwrap();
        assert_eq!(
            ready.fact,
            LifecycleFact::Ready {
                started_at_ms: 10,
                end_at_ms: 60_010,
                max_end_at_ms: 300_010
            }
        );
        assert_eq!(launches(&state), 1);
    }
    clock.0.store(100, Ordering::SeqCst);
    let app = local::application(&path, Arc::clone(&state), Arc::clone(&clock), false);
    let service = HostLifecycle::new(app);
    service.recover().await.unwrap();
    assert_eq!(
        service
            .advance(create.clone(), None, local::compute())
            .await
            .unwrap(),
        ready
    );
    assert_eq!(launches(&state), 1);
    let stop = destroy(&create);
    service.receive(stop.clone()).await.unwrap();
    state.lock().unwrap().release_unknown = true;
    let stopped = service
        .advance(stop.clone(), None, local::compute())
        .await
        .unwrap();
    assert_eq!(stopped.fact, LifecycleFact::Stopped);
    assert_eq!(service.receive(stop.clone()).await.unwrap(), stopped);
    state.lock().unwrap().release_unknown = false;
    let cleaned = service
        .advance(stop.clone(), None, local::compute())
        .await
        .unwrap();
    assert_eq!(cleaned.fact, LifecycleFact::Cleaned);
    assert!(cleaned.observation_version.get() > stopped.observation_version.get());
    assert_eq!(service.receive(stop).await.unwrap(), cleaned);
    assert_eq!(
        service
            .advance(create, None, local::compute())
            .await
            .unwrap(),
        ready
    );
    assert_eq!(launches(&state), 1);
}

#[tokio::test]
async fn lost_activation_reply_recovers_original_vm_and_original_configuration() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("db");
    let state = Arc::new(Mutex::new(local::State {
        lose_activation: true,
        ..local::State::default()
    }));
    let clock = Arc::new(local::TestClock(AtomicI64::new(10)));
    let create = local::request();
    let credential;
    {
        let service = HostLifecycle::new(local::application(
            &path,
            Arc::clone(&state),
            Arc::clone(&clock),
            true,
        ));
        service.receive(create.clone()).await.unwrap();
        let response = service
            .advance(create.clone(), Some(local::template()), local::compute())
            .await
            .unwrap();
        assert_eq!(response.fact, LifecycleFact::Recorded);
        credential = state.lock().unwrap().configurations.clone();
    }
    clock.0.store(20, Ordering::SeqCst);
    let service = HostLifecycle::new(local::application(&path, Arc::clone(&state), clock, false));
    let ready = service
        .advance(create, None, local::compute())
        .await
        .unwrap();
    assert_eq!(
        ready.fact,
        LifecycleFact::Ready {
            started_at_ms: 20,
            end_at_ms: 60_020,
            max_end_at_ms: 300_020
        }
    );
    assert_eq!(launches(&state), 1);
    assert_eq!(state.lock().unwrap().configurations, credential);
}

#[tokio::test]
async fn destroy_before_admission_prevents_late_create_and_cached_template_reuse() {
    let dir = tempfile::tempdir().unwrap();
    let state = Arc::new(Mutex::new(local::State::default()));
    let app = local::application(
        &dir.path().join("db"),
        Arc::clone(&state),
        Arc::new(local::TestClock(AtomicI64::new(10))),
        true,
    );
    let service = HostLifecycle::new(Arc::clone(&app));
    let create = local::request();
    let recorded = service.receive(create.clone()).await.unwrap();
    let stop = destroy(&create);
    service.receive(stop.clone()).await.unwrap();
    assert_eq!(
        service
            .advance(stop.clone(), None, local::compute())
            .await
            .unwrap()
            .fact,
        LifecycleFact::Cleaned
    );
    assert_eq!(
        service
            .advance(create, Some(local::template()), local::compute())
            .await
            .unwrap(),
        recorded
    );
    assert_eq!(launches(&state), 0);
    assert_eq!(
        app.internal_pending(&fixture::node("host-a")).unwrap(),
        vec![stop]
    );
}

#[tokio::test]
async fn disconnected_center_cannot_prevent_original_host_expiry_or_extend_lifetime() {
    let dir = tempfile::tempdir().unwrap();
    let state = Arc::new(Mutex::new(local::State::default()));
    let clock = Arc::new(local::TestClock(AtomicI64::new(10)));
    let service = HostLifecycle::new(local::application(
        &dir.path().join("db"),
        Arc::clone(&state),
        Arc::clone(&clock),
        true,
    ));
    let create = local::request();
    service.receive(create.clone()).await.unwrap();
    service
        .advance(create.clone(), Some(local::template()), local::compute())
        .await
        .unwrap();
    clock.0.store(60_010, Ordering::SeqCst);
    let (_, expiry) = service.recover().await.unwrap();
    assert_eq!(expiry.completed, 1);
    assert_eq!(launches(&state), 1);
    let stop = destroy(&create);
    service.receive(stop.clone()).await.unwrap();
    assert_eq!(
        service
            .advance(stop, None, local::compute())
            .await
            .unwrap()
            .fact,
        LifecycleFact::Cleaned
    );
}

#[tokio::test]
async fn destroy_reception_revokes_data_before_worker_stop_and_waits_for_original_drain() {
    use sandproof_controller_service::application::{ApplicationError, DataKind};
    let dir = tempfile::tempdir().unwrap();
    let state = Arc::new(Mutex::new(local::State::default()));
    let app = local::application(
        &dir.path().join("db"),
        Arc::clone(&state),
        Arc::new(local::TestClock(AtomicI64::new(10))),
        true,
    );
    let service = HostLifecycle::new(Arc::clone(&app));
    let create = local::request();
    service.receive(create.clone()).await.unwrap();
    service
        .advance(create.clone(), Some(local::template()), local::compute())
        .await
        .unwrap();
    let credential = state
        .lock()
        .unwrap()
        .configurations
        .values()
        .next()
        .unwrap()
        .0
        .clone();
    let permit = app.data_permit(&credential, DataKind::Exec).unwrap();
    assert!(!*permit.revocation.borrow());
    let stop = destroy(&create);
    service.receive(stop.clone()).await.unwrap();
    assert!(*permit.revocation.borrow());
    assert_eq!(
        app.authorize_data(&credential, DataKind::Exec),
        Err(ApplicationError::NotFound)
    );
    assert_eq!(
        service
            .advance(stop.clone(), None, local::compute())
            .await
            .unwrap()
            .fact,
        LifecycleFact::Stopped
    );
    drop(permit);
    assert_eq!(
        service
            .advance(stop, None, local::compute())
            .await
            .unwrap()
            .fact,
        LifecycleFact::Cleaned
    );
}
