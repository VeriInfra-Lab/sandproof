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

fn renew(
    create: &sandproof_protocol::internal::lifecycle::LifecycleRequest,
    end_at_ms: u64,
) -> sandproof_protocol::internal::lifecycle::LifecycleRequest {
    let mut request = destroy(create);
    request.intent = Intent::Renew { end_at_ms };
    request
}

#[tokio::test]
async fn definitive_original_pre_admission_capacity_veto_is_durable_not_accepted() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("db");
    let state = Arc::new(Mutex::new(local::State {
        disk_available: Some(0),
        ..local::State::default()
    }));
    let clock = Arc::new(local::TestClock(AtomicI64::new(10)));
    let create = local::request();
    let result;
    {
        let service = HostLifecycle::new(local::application(
            &path,
            Arc::clone(&state),
            Arc::clone(&clock),
            true,
        ));
        service.receive(create.clone()).await.unwrap();
        result = service
            .advance(create.clone(), Some(local::template()), local::compute())
            .await
            .unwrap();
        assert_eq!(result.fact, LifecycleFact::NotAccepted);
        assert_eq!(launches(&state), 0);
        assert!(state.lock().unwrap().configurations.is_empty());
    }
    // Restored free space must not silently re-admit this original refused
    // identity. A later fresh center intent uses its own retired lane generation.
    state.lock().unwrap().disk_available = None;
    let app = local::application(&path, Arc::clone(&state), clock, false);
    assert!(
        app.internal_pending(&create.identity.node)
            .unwrap()
            .is_empty()
    );
    let service = HostLifecycle::new(app);
    assert_eq!(service.receive(create.clone()).await.unwrap(), result);
    assert_eq!(
        service
            .advance(create, None, local::compute())
            .await
            .unwrap(),
        result
    );
    assert_eq!(launches(&state), 0);
    assert!(state.lock().unwrap().configurations.is_empty());
}

#[tokio::test]
async fn missing_or_mismatched_material_stays_recorded_never_capacity_rejected() {
    use sandproof_controller_service::application::ApplicationError;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("db");
    let state = Arc::new(Mutex::new(local::State {
        disk_available: Some(0),
        ..local::State::default()
    }));
    let clock = Arc::new(local::TestClock(AtomicI64::new(10)));
    let app = local::application(&path, Arc::clone(&state), clock, true);
    let service = HostLifecycle::new(app);
    let create = local::request();
    let original = service.receive(create.clone()).await.unwrap();
    assert_eq!(
        service
            .advance(create.clone(), None, local::compute())
            .await,
        Err(ApplicationError::Busy)
    );
    let mut changed = local::template();
    changed.digest = [9; 32];
    assert_eq!(
        service
            .advance(create.clone(), Some(changed), local::compute())
            .await,
        Err(ApplicationError::Invalid)
    );
    assert_eq!(service.receive(create.clone()).await.unwrap(), original);
    // Even definitive low disk cannot turn a foreign registered private owner
    // or changed immutable resource shape into a capacity acceptance fact.
    for private_owner in [true, false] {
        let mut changed = local::template();
        if private_owner {
            changed.registered.owner = Some(2);
            changed.local.owner = Some(2);
        } else {
            changed.registered.resources.cpu_count = 2;
            changed.local.resources.cpu_count = 2;
        }
        changed.digest = changed.registered.canonical_digest().unwrap();
        let error = service
            .advance(create.clone(), Some(changed), local::compute())
            .await
            .unwrap_err();
        assert_eq!(error, ApplicationError::Invalid);
        assert_eq!(service.receive(create.clone()).await.unwrap(), original);
    }
    // A callback error cannot masquerade as the original numeric capacity
    // decision, even if its coarse application category also says quota.
    state.lock().unwrap().template_error = Some(ApplicationError::QuotaExceeded);
    assert_eq!(
        service
            .advance(create.clone(), Some(local::template()), local::compute())
            .await,
        Err(ApplicationError::QuotaExceeded)
    );
    assert_eq!(service.receive(create).await.unwrap(), original);
    assert_eq!(launches(&state), 0);
}

#[tokio::test]
async fn original_native_compute_quota_refuses_peer_without_changing_live_instance() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("db");
    let state = Arc::new(Mutex::new(local::State::default()));
    let app = local::application(
        &path,
        Arc::clone(&state),
        Arc::new(local::TestClock(AtomicI64::new(10))),
        true,
    );
    let service = HostLifecycle::new(Arc::clone(&app));
    let first = local::request();
    let mut compute = local::compute();
    compute.max_vcpus_per_tenant = 1;
    service.receive(first.clone()).await.unwrap();
    let running = service
        .advance(first.clone(), Some(local::template()), compute)
        .await
        .unwrap();
    assert!(matches!(running.fact, LifecycleFact::Ready { .. }));
    let actions = state.lock().unwrap().actions.clone();
    let mut peer = first.clone();
    peer.identity.retention_slot = 2;
    peer.identity.sandbox_id = "sandbox-2".try_into().unwrap();
    peer.identity.operation_id = "operation-2".try_into().unwrap();
    service.receive(peer.clone()).await.unwrap();
    let refused = service
        .advance(peer.clone(), Some(local::template()), compute)
        .await
        .unwrap();
    assert_eq!(refused.fact, LifecycleFact::NotAccepted);
    assert_eq!(state.lock().unwrap().actions, actions);
    assert_eq!(app.internal_audit().unwrap().native.len(), 1);
    assert_eq!(service.receive(first).await.unwrap(), running);
    assert_eq!(service.advance(peer, None, compute).await.unwrap(), refused);
}

#[tokio::test]
async fn registered_builder_uses_original_internal_admission_without_private_window_metadata() {
    use sandproof_controller_service::physical_storage::{PhysicalBuildStorage, PoolRole};
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("db");
    let state = Arc::new(Mutex::new(local::State::default()));
    let reserve = local::complete_fixture::CLEANUP_BYTES + 1000;
    let namespace = "0123456789abcdef";
    let mut host = local::complete_fixture::host(&path, namespace, 64, 2 * 1024 * 1024, reserve);
    let profile = host.physical_storage.as_mut().unwrap();
    profile.pools[0].role = PoolRole::Builder;
    let mut runtime = profile.pools[0].clone();
    runtime.role = PoolRole::Runtime;
    runtime.namespace = "1123456789abcdef".into();
    runtime.database = directory.path().join("runtime-db");
    runtime.node_state_dir = directory.path().join("runtime-node");
    runtime.chroot_base = directory.path().join("runtime-jails");
    profile.pools.push(runtime);
    profile.public_build = Some(PhysicalBuildStorage {
        workspace_owner_uid: 0,
        registry_owner_uid: 0,
        root: directory.path().join("build-work"),
        registry_root: directory.path().join("center-registry"),
        workspace_bytes: 4096,
        registry_bytes: 4096,
        max_jobs: 1,
        max_templates: 1,
        inode_budget: 32,
    });
    profile.filesystem_overhead_inodes = 4096;
    let inbox = sandproof_controller_service::InternalInboxConfig {
        node: fixture::node("host-a"),
        max_sandboxes: 2,
        operations_per_sandbox: 4,
        max_options_bytes: 1024,
    };
    let app = local::application_with_host_config(
        &path,
        Arc::clone(&state),
        Arc::new(local::TestClock(AtomicI64::new(10))),
        true,
        inbox,
        host.clone(),
    );
    let service = HostLifecycle::new(app);
    let create = local::request();
    service.receive(create.clone()).await.unwrap();
    assert!(matches!(
        service
            .advance(create, Some(local::template()), local::compute())
            .await
            .unwrap()
            .fact,
        LifecycleFact::Ready { .. }
    ));
    assert_eq!(launches(&state), 1);
    assert!(
        state
            .lock()
            .unwrap()
            .configurations
            .values()
            .all(|(_, options)| options.metadata.is_empty())
    );
    for missing in [false, true] {
        let mut changed = host.clone();
        let pool = &mut changed.physical_storage.as_mut().unwrap().pools[0];
        if missing {
            pool.namespace = "2123456789abcdef".into();
        } else {
            pool.role = PoolRole::Runtime;
        }
        assert!(
            sandproof_controller_service::ControllerJournal::preflight_complete(
                &path,
                namespace,
                &changed,
                &local::complete_fixture::maintenance()
            )
            .is_err()
        );
    }
    assert_eq!(launches(&state), 1);
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
        vec![]
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

#[tokio::test]
async fn fixed_renewal_reopen_replay_retains_original_vm_and_host_expiry() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("db");
    let state = Arc::new(Mutex::new(local::State::default()));
    let clock = Arc::new(local::TestClock(AtomicI64::new(10)));
    let create = local::request();
    let renewal = renew(&create, 90_010);
    let renewed;
    {
        let service = HostLifecycle::new(local::application(
            &path,
            Arc::clone(&state),
            Arc::clone(&clock),
            true,
        ));
        service.receive(create.clone()).await.unwrap();
        service
            .advance(create, Some(local::template()), local::compute())
            .await
            .unwrap();
        clock.0.store(100, Ordering::SeqCst);
        service.receive(renewal.clone()).await.unwrap();
        assert_eq!(
            service.pending(fixture::node("host-a")).await.unwrap(),
            vec![renewal.clone()]
        );
        renewed = service
            .advance(renewal.clone(), None, local::compute())
            .await
            .unwrap();
        assert_eq!(renewed.fact, LifecycleFact::Renewed { end_at_ms: 90_010 });
    }
    clock.0.store(90_010, Ordering::SeqCst);
    let service = HostLifecycle::new(local::application(
        &path,
        Arc::clone(&state),
        Arc::clone(&clock),
        false,
    ));
    assert_eq!(service.receive(renewal.clone()).await.unwrap(), renewed);
    assert_eq!(
        service
            .advance(renewal, None, local::compute())
            .await
            .unwrap(),
        renewed
    );
    let (_, expiry) = service.recover().await.unwrap();
    assert_eq!(expiry.completed, 1);
    assert_eq!(launches(&state), 1);
}

#[tokio::test]
async fn immediate_fixed_expiry_revokes_data_without_waiting_for_physical_stop() {
    use sandproof_controller_service::application::{ApplicationError, DataKind};
    let dir = tempfile::tempdir().unwrap();
    let state = Arc::new(Mutex::new(local::State::default()));
    let clock = Arc::new(local::TestClock(AtomicI64::new(10)));
    let app = local::application(
        &dir.path().join("db"),
        Arc::clone(&state),
        Arc::clone(&clock),
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
    clock.0.store(100, Ordering::SeqCst);
    let renewal = renew(&create, 100);
    service.receive(renewal.clone()).await.unwrap();
    assert!(!*permit.revocation.borrow());
    assert_eq!(
        service
            .advance(renewal, None, local::compute())
            .await
            .unwrap()
            .fact,
        LifecycleFact::Renewed { end_at_ms: 100 }
    );
    assert!(*permit.revocation.borrow());
    assert_eq!(
        app.authorize_data(&credential, DataKind::Exec),
        Err(ApplicationError::NotFound)
    );
    assert_eq!(launches(&state), 1);
    drop(permit);
    let (_, expiry) = service.recover().await.unwrap();
    assert_eq!(expiry.completed, 1);
}
