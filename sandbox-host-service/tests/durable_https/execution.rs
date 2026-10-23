//! Original current application/SQLite with controlled physical effects.
use super::{Arc, AtomicI64, Mutex, local};
use sandproof_host_service::HostLifecycle;
use sandproof_internal::LifecycleReceiver;
use sandproof_protocol::lifecycle::{CreateRefusalReason, CurrentCreateReadiness, TerminalStatus};
use std::sync::atomic::Ordering;

pub(super) fn launches(state: &Mutex<local::State>) -> usize {
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
async fn ready(
    host: &HostLifecycle<local::TestHost>,
    create: &sandproof_protocol::lifecycle::AcceptedLifecycleIntent,
) -> sandproof_protocol::lifecycle::CurrentReadyFacts {
    let observation = host
        .observe_create(create.clone(), local::context())
        .await
        .unwrap();
    assert!(observation.matches_request(create));
    let CurrentCreateReadiness::Observed { facts } = observation.readiness else {
        panic!("original VM is not currently Ready")
    };
    facts
}

#[tokio::test]
async fn definitive_original_pre_admission_capacity_veto_is_durable_not_accepted() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("db");
    let state = Arc::new(Mutex::new(local::State {
        disk_available: Some(0),
        ..local::state()
    }));
    let clock = Arc::new(local::TestClock(AtomicI64::new(10)));
    let create = local::request();
    let refused;
    {
        let host = HostLifecycle::new(local::application(
            &path,
            state.clone(),
            clock.clone(),
            true,
        ));
        host.record_lifecycle(create.clone(), local::context())
            .await
            .unwrap();
        refused = host
            .advance(create.clone(), Some(local::template()), local::compute())
            .await
            .unwrap();
        assert_eq!(refused.refusal, Some(CreateRefusalReason::Capacity));
        assert!(refused.local.is_none());
        assert_eq!(launches(&state), 0);
        assert!(state.lock().unwrap().configurations.is_empty());
    }
    state.lock().unwrap().disk_available = None;
    let app = local::application(&path, state.clone(), clock, false);
    assert!(
        app.lifecycle_pending_creates(&local::config().node)
            .unwrap()
            .is_empty()
    );
    let host = HostLifecycle::new(app);
    assert!(
        host.record_lifecycle(create.clone(), local::context())
            .await
            .unwrap()
            .matches(&create)
    );
    assert_eq!(
        host.advance(create.clone(), None, local::compute())
            .await
            .unwrap(),
        refused
    );
    assert_eq!(
        host.observe_create(create, local::context())
            .await
            .unwrap()
            .readiness,
        CurrentCreateReadiness::Refused {
            reason: CreateRefusalReason::Capacity
        }
    );
    assert_eq!(launches(&state), 0);
    assert!(state.lock().unwrap().configurations.is_empty());
}

#[tokio::test]
async fn missing_or_mismatched_material_stays_recorded_never_capacity_rejected() {
    use sandproof_controller_service::application::ApplicationError;
    let dir = tempfile::tempdir().unwrap();
    let state = Arc::new(Mutex::new(local::State {
        disk_available: Some(0),
        ..local::state()
    }));
    let clock = Arc::new(local::TestClock(AtomicI64::new(10)));
    let app = local::application(&dir.path().join("db"), state.clone(), clock, true);
    let host = HostLifecycle::new(app.clone());
    let create = local::request();
    let original = host
        .record_lifecycle(create.clone(), local::context())
        .await
        .unwrap();
    assert_eq!(
        host.advance(create.clone(), None, local::compute()).await,
        Err(ApplicationError::Busy)
    );
    let mut changed = local::template();
    changed.digest = [9; 32];
    assert_eq!(
        host.advance(create.clone(), Some(changed), local::compute())
            .await,
        Err(ApplicationError::Invalid)
    );
    assert_eq!(
        host.record_lifecycle(create.clone(), local::context())
            .await
            .unwrap(),
        original
    );
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
        assert_eq!(
            host.advance(create.clone(), Some(changed), local::compute())
                .await
                .unwrap_err(),
            ApplicationError::Invalid
        );
        assert_eq!(
            host.record_lifecycle(create.clone(), local::context())
                .await
                .unwrap(),
            original
        );
    }
    state.lock().unwrap().template_error = Some(ApplicationError::QuotaExceeded);
    assert_eq!(
        host.advance(create.clone(), Some(local::template()), local::compute())
            .await,
        Err(ApplicationError::QuotaExceeded)
    );
    assert_eq!(
        host.record_lifecycle(create.clone(), local::context())
            .await
            .unwrap(),
        original
    );
    assert!(
        app.lifecycle_work_state(&create, &local::context())
            .unwrap()
            .refusal
            .is_none()
    );
    assert_eq!(launches(&state), 0);
}

#[tokio::test]
async fn original_native_compute_quota_refuses_peer_without_changing_live_instance() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("db");
    let state = Arc::new(Mutex::new(local::state()));
    let app = local::application(
        &path,
        state.clone(),
        Arc::new(local::TestClock(AtomicI64::new(10))),
        true,
    );
    let host = HostLifecycle::new(app);
    let first = local::request();
    let mut compute = local::compute();
    compute.max_vcpus_per_tenant = 1;
    host.record_lifecycle(first.clone(), local::context())
        .await
        .unwrap();
    host.advance(first.clone(), Some(local::template()), compute)
        .await
        .unwrap();
    let running = ready(&host, &first).await;
    let actions = state.lock().unwrap().actions.clone();
    let mut peer = first.clone();
    peer.retention.slot = 2;
    peer.request.binding.session = "session-2".try_into().unwrap();
    peer.request.binding.key.sequence = 2.try_into().unwrap();
    host.record_lifecycle(peer.clone(), local::context())
        .await
        .unwrap();
    let refused = host
        .advance(peer.clone(), Some(local::template()), compute)
        .await
        .unwrap();
    assert_eq!(refused.refusal, Some(CreateRefusalReason::Capacity));
    assert_eq!(state.lock().unwrap().actions, actions);
    assert_eq!(local::audit(&path).native.len(), 1);
    assert_eq!(ready(&host, &first).await, running);
    assert_eq!(host.advance(peer, None, compute).await.unwrap(), refused);
}

#[tokio::test]
async fn registered_builder_uses_original_internal_admission_without_private_window_metadata() {
    use sandproof_controller_service::physical_storage::{PhysicalBuildStorage, PoolRole};
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("db");
    let state = Arc::new(Mutex::new(local::state()));
    let reserve = local::complete_fixture::CLEANUP_BYTES + 1000;
    let namespace = "0123456789abcdef";
    let mut host_config =
        local::complete_fixture::host(&path, namespace, 64, 2 * 1024 * 1024, reserve);
    let profile = host_config.physical_storage.as_mut().unwrap();
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
    let app = local::application_with_host_config(
        &path,
        state.clone(),
        Arc::new(local::TestClock(AtomicI64::new(10))),
        true,
        host_config.clone(),
    );
    let host = HostLifecycle::new(app);
    let create = local::request();
    host.record_lifecycle(create.clone(), local::context())
        .await
        .unwrap();
    host.advance(create.clone(), Some(local::template()), local::compute())
        .await
        .unwrap();
    ready(&host, &create).await;
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
        let mut changed = host_config.clone();
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
                &local::current::maintenance()
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
    let state = Arc::new(Mutex::new(local::state()));
    let clock = Arc::new(local::TestClock(AtomicI64::new(10)));
    let create = local::request();
    let running;
    {
        let host = HostLifecycle::new(local::application(
            &path,
            state.clone(),
            clock.clone(),
            true,
        ));
        let receipt = host
            .record_lifecycle(create.clone(), local::context())
            .await
            .unwrap();
        assert!(receipt.matches(&create));
        assert_eq!(launches(&state), 0);
        host.advance(create.clone(), Some(local::template()), local::compute())
            .await
            .unwrap();
        running = ready(&host, &create).await;
        assert_eq!(
            (
                running.started_at_ms,
                running.end_at_ms,
                running.max_end_at_ms
            ),
            (10, 60_010, 300_010)
        );
        assert_eq!(launches(&state), 1);
    }
    clock.0.store(100, Ordering::SeqCst);
    let host = HostLifecycle::new(local::application(&path, state.clone(), clock, false));
    host.recover().await.unwrap();
    host.advance(create.clone(), None, local::compute())
        .await
        .unwrap();
    let current = ready(&host, &create).await;
    assert_eq!(
        (
            current.started_at_ms,
            current.end_at_ms,
            current.max_end_at_ms
        ),
        (
            running.started_at_ms,
            running.end_at_ms,
            running.max_end_at_ms
        )
    );
    assert_eq!(launches(&state), 1);
    let stop = local::destroy(&create);
    host.record_lifecycle(stop.clone(), local::context())
        .await
        .unwrap();
    state.lock().unwrap().release_unknown = true;
    local::drive(&host).await;
    assert_eq!(
        host.observe_terminal(stop.clone(), local::context())
            .await
            .unwrap()
            .status,
        TerminalStatus::Pending
    );
    assert!(state.lock().unwrap().released_slots.is_empty());
    state.lock().unwrap().release_unknown = false;
    local::drive(&host).await;
    let cleaned = host
        .observe_terminal(stop.clone(), local::context())
        .await
        .unwrap();
    assert_eq!(cleaned.status, TerminalStatus::Cleaned);
    assert_eq!(
        host.observe_terminal(stop, local::context()).await.unwrap(),
        cleaned
    );
    let original = host.observe_create(create, local::context()).await.unwrap();
    assert!(original.historical_create_succeeded);
    assert_eq!(original.readiness, CurrentCreateReadiness::NotCurrent);
    assert_eq!(state.lock().unwrap().released_slots.len(), 1);
    assert_eq!(launches(&state), 1);
}

#[tokio::test]
async fn lost_activation_reply_recovers_original_vm_and_original_configuration() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("db");
    let state = Arc::new(Mutex::new(local::State {
        lose_activation: true,
        ..local::state()
    }));
    let clock = Arc::new(local::TestClock(AtomicI64::new(10)));
    let create = local::request();
    let credential;
    {
        let host = HostLifecycle::new(local::application(
            &path,
            state.clone(),
            clock.clone(),
            true,
        ));
        host.record_lifecycle(create.clone(), local::context())
            .await
            .unwrap();
        host.advance(create.clone(), Some(local::template()), local::compute())
            .await
            .unwrap();
        assert_eq!(
            host.observe_create(create.clone(), local::context())
                .await
                .unwrap()
                .readiness,
            CurrentCreateReadiness::Pending
        );
        credential = state.lock().unwrap().configurations.clone();
    }
    clock.0.store(20, Ordering::SeqCst);
    let host = HostLifecycle::new(local::application(&path, state.clone(), clock, false));
    host.advance(create.clone(), None, local::compute())
        .await
        .unwrap();
    let facts = ready(&host, &create).await;
    assert_eq!(
        (facts.started_at_ms, facts.end_at_ms, facts.max_end_at_ms),
        (20, 60_020, 300_020)
    );
    assert_eq!(launches(&state), 1);
    assert_eq!(state.lock().unwrap().configurations, credential);
}

#[tokio::test]
async fn destroy_before_admission_prevents_late_create_and_cached_template_reuse() {
    let dir = tempfile::tempdir().unwrap();
    let state = Arc::new(Mutex::new(local::state()));
    let app = local::application(
        &dir.path().join("db"),
        state.clone(),
        Arc::new(local::TestClock(AtomicI64::new(10))),
        true,
    );
    let host = HostLifecycle::new(app.clone());
    let create = local::request();
    host.record_lifecycle(create.clone(), local::context())
        .await
        .unwrap();
    let stop = local::destroy(&create);
    host.record_lifecycle(stop.clone(), local::context())
        .await
        .unwrap();
    local::drive(&host).await;
    assert_eq!(
        host.observe_terminal(stop, local::context())
            .await
            .unwrap()
            .status,
        TerminalStatus::Cleaned
    );
    let before = app
        .lifecycle_work_state(&create, &local::context())
        .unwrap();
    assert_eq!(
        host.advance(create, Some(local::template()), local::compute())
            .await
            .unwrap(),
        before
    );
    assert_eq!(launches(&state), 0);
    assert!(
        app.lifecycle_pending_creates(&local::config().node)
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn disconnected_center_cannot_prevent_original_host_expiry_or_extend_lifetime() {
    let dir = tempfile::tempdir().unwrap();
    let state = Arc::new(Mutex::new(local::state()));
    let clock = Arc::new(local::TestClock(AtomicI64::new(10)));
    let host = HostLifecycle::new(local::application(
        &dir.path().join("db"),
        state.clone(),
        clock.clone(),
        true,
    ));
    let create = local::request();
    host.record_lifecycle(create.clone(), local::context())
        .await
        .unwrap();
    host.advance(create.clone(), Some(local::template()), local::compute())
        .await
        .unwrap();
    clock.0.store(60_010, Ordering::SeqCst);
    host.recover().await.unwrap();
    assert_eq!(launches(&state), 1);
    assert_eq!(state.lock().unwrap().released_slots.len(), 1);
    let stop = local::destroy(&create);
    host.record_lifecycle(stop.clone(), local::context())
        .await
        .unwrap();
    local::drive(&host).await;
    assert_eq!(
        host.observe_terminal(stop, local::context())
            .await
            .unwrap()
            .status,
        TerminalStatus::Cleaned
    );
}

#[tokio::test]
async fn destroy_reception_revokes_data_before_worker_stop_and_waits_for_original_drain() {
    use sandproof_controller_service::application::{ApplicationError, DataKind};
    let dir = tempfile::tempdir().unwrap();
    let state = Arc::new(Mutex::new(local::state()));
    let app = local::application(
        &dir.path().join("db"),
        state.clone(),
        Arc::new(local::TestClock(AtomicI64::new(10))),
        true,
    );
    let host = HostLifecycle::new(app.clone());
    let create = local::request();
    host.record_lifecycle(create.clone(), local::context())
        .await
        .unwrap();
    host.advance(create.clone(), Some(local::template()), local::compute())
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
    let stop = local::destroy(&create);
    host.record_lifecycle(stop.clone(), local::context())
        .await
        .unwrap();
    assert!(*permit.revocation.borrow());
    assert_eq!(
        app.authorize_data(&credential, DataKind::Exec),
        Err(ApplicationError::NotFound)
    );
    local::drive(&host).await;
    assert_eq!(
        host.observe_terminal(stop.clone(), local::context())
            .await
            .unwrap()
            .status,
        TerminalStatus::Pending
    );
    assert!(state.lock().unwrap().released_slots.is_empty());
    drop(permit);
    local::drive(&host).await;
    assert_eq!(
        host.observe_terminal(stop, local::context())
            .await
            .unwrap()
            .status,
        TerminalStatus::Cleaned
    );
    assert_eq!(state.lock().unwrap().released_slots.len(), 1);
}

#[tokio::test]
async fn fixed_renewal_reopen_replay_retains_original_vm_and_host_expiry() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("db");
    let state = Arc::new(Mutex::new(local::state()));
    let clock = Arc::new(local::TestClock(AtomicI64::new(10)));
    let create = local::request();
    let renewal = local::renew(&create, 90_010);
    let renewed;
    {
        let app = local::application(&path, state.clone(), clock.clone(), true);
        let host = HostLifecycle::new(app.clone());
        host.record_lifecycle(create.clone(), local::context())
            .await
            .unwrap();
        host.advance(create.clone(), Some(local::template()), local::compute())
            .await
            .unwrap();
        clock.0.store(100, Ordering::SeqCst);
        host.record_lifecycle(renewal.clone(), local::context())
            .await
            .unwrap();
        renewed = app
            .apply_lifecycle_renew(&renewal, &local::context())
            .unwrap();
        assert_eq!(renewed.1, 90_010);
        assert!(
            app.lifecycle_pending_renewals(&local::config().node)
                .unwrap()
                .is_empty()
        );
    }
    clock.0.store(90_010, Ordering::SeqCst);
    let app = local::application(&path, state.clone(), clock, false);
    let host = HostLifecycle::new(app.clone());
    host.record_lifecycle(renewal.clone(), local::context())
        .await
        .unwrap();
    assert_eq!(
        app.apply_lifecycle_renew(&renewal, &local::context())
            .unwrap(),
        renewed
    );
    host.recover().await.unwrap();
    assert_eq!(state.lock().unwrap().released_slots.len(), 1);
    assert_eq!(launches(&state), 1);
}

#[tokio::test]
async fn immediate_fixed_expiry_revokes_data_without_waiting_for_physical_stop() {
    use sandproof_controller_service::application::{ApplicationError, DataKind};
    let dir = tempfile::tempdir().unwrap();
    let state = Arc::new(Mutex::new(local::state()));
    let clock = Arc::new(local::TestClock(AtomicI64::new(10)));
    let app = local::application(&dir.path().join("db"), state.clone(), clock.clone(), true);
    let host = HostLifecycle::new(app.clone());
    let create = local::request();
    host.record_lifecycle(create.clone(), local::context())
        .await
        .unwrap();
    host.advance(create.clone(), Some(local::template()), local::compute())
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
    clock.0.store(100, Ordering::SeqCst);
    let renewal = local::renew(&create, 100);
    // Current reception applies the exact normalized update before returning.
    host.record_lifecycle(renewal.clone(), local::context())
        .await
        .unwrap();
    assert_eq!(
        app.apply_lifecycle_renew(&renewal, &local::context())
            .unwrap()
            .1,
        100
    );
    assert!(*permit.revocation.borrow());
    assert_eq!(
        app.authorize_data(&credential, DataKind::Exec),
        Err(ApplicationError::NotFound)
    );
    assert_eq!(launches(&state), 1);
    assert!(state.lock().unwrap().released_slots.is_empty());
    drop(permit);
    host.recover().await.unwrap();
    assert_eq!(state.lock().unwrap().released_slots.len(), 1);
}
