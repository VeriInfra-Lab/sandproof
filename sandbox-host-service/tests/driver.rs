//! Real original SQLite/application; controlled physical effects, not KVM.
#[allow(dead_code, reason = "shared original application fixture")]
#[path = "../../controller-service/tests/support/internal_application.rs"]
mod local;
use sandproof_controller_service::{InternalTemplate, application::ApplicationError};
use sandproof_host_service::{DriverConfig, HostLifecycle, TemplateSource};
use sandproof_internal::{
    LifecycleReceiver,
    status::{HostStatus, StatusRequest},
};
use sandproof_protocol::internal::lifecycle::{CreateBinding, Intent, LifecycleFact};
use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicI64, Ordering},
    },
    time::Duration,
};

struct Templates;
impl TemplateSource for Templates {
    fn bound(&self, binding: &CreateBinding) -> Result<InternalTemplate, ApplicationError> {
        let template = local::template();
        if template.revision != binding.template_revision
            || template.digest != binding.template_digest
        {
            return Err(ApplicationError::NotFound);
        }
        Ok(template)
    }
}
struct Missing;
impl TemplateSource for Missing {
    fn bound(&self, _: &CreateBinding) -> Result<InternalTemplate, ApplicationError> {
        Err(ApplicationError::NotFound)
    }
}

#[tokio::test]
async fn health_waits_once_behind_original_work_then_observes_the_current_resources() {
    use std::{future::Future, sync::mpsc, task::Poll};
    let dir = tempfile::tempdir().unwrap();
    let (entered, observed) = mpsc::channel();
    let (release, blocked) = mpsc::channel();
    let state = Arc::new(Mutex::new(local::State {
        block_prepare: Some((entered, blocked)),
        ..local::State::default()
    }));
    let host = HostLifecycle::new(local::application(
        &dir.path().join("db"),
        state,
        Arc::new(local::TestClock(AtomicI64::new(10))),
        true,
    ));
    let create = local::request();
    host.receive(create.clone()).await.unwrap();
    let original = host.clone();
    let work = tokio::spawn(async move {
        original
            .advance(create, Some(local::template()), local::compute())
            .await
    });
    tokio::task::spawn_blocking(move || observed.recv_timeout(Duration::from_secs(5)).unwrap())
        .await
        .unwrap();
    let mut health = Box::pin(host.free_resources(config().node.clone(), config().compute));
    let pending =
        std::future::poll_fn(|cx| Poll::Ready(health.as_mut().poll(cx).is_pending())).await;
    // Release the actual worker even when the regression fails on old code.
    release.send(()).unwrap();
    assert!(
        pending,
        "health was dropped as Busy instead of reserving the next work slot"
    );
    assert!(
        matches!(
            host.free_resources(config().node.clone(), config().compute)
                .await,
            Err(ApplicationError::Busy)
        ),
        "a second observation formed another waiter"
    );
    let ready = work.await.unwrap().unwrap();
    assert!(matches!(ready.fact, LifecycleFact::Ready { .. }));
    let (observed_at, free) = health.await.unwrap();
    assert_eq!(observed_at, 10);
    assert_eq!(free.instances, 1); // Real committed post-create usage, not cached health.
    assert_eq!(host.active_work(), 0);
}

struct CorruptAuthority(std::path::PathBuf);
impl TemplateSource for CorruptAuthority {
    fn bound(&self, _: &CreateBinding) -> Result<InternalTemplate, ApplicationError> {
        // Foreign modification is only a test fault injection, never a second
        // production writer. It happens after the finite durable scan but before
        // the original application admission/effect edges.
        rusqlite::Connection::open(&self.0)
            .unwrap()
            .execute(
                "UPDATE controller_internal_inbox SET sha256=zeroblob(32) WHERE id=1",
                [],
            )
            .unwrap();
        Ok(local::template())
    }
}

#[tokio::test]
async fn authority_fault_during_advance_halts_before_another_healthy_request() {
    let dir = tempfile::tempdir().unwrap();
    let database = dir.path().join("db");
    let state = Arc::new(Mutex::new(local::State::default()));
    let app = local::application(
        &database,
        Arc::clone(&state),
        Arc::new(local::TestClock(AtomicI64::new(10))),
        true,
    );
    let host = HostLifecycle::new(app);
    host.receive(local::request()).await.unwrap();
    let mut healthy = local::request();
    healthy.identity.sandbox_id = "sandbox-2".try_into().unwrap();
    healthy.identity.operation_id = "operation-2".try_into().unwrap();
    healthy.identity.retention_slot = 2;
    host.receive(healthy).await.unwrap();
    assert_eq!(
        host.drive_once(config(), Arc::new(CorruptAuthority(database)))
            .await,
        Err(ApplicationError::OutcomeUnknown)
    );
    assert!(state.lock().unwrap().actions.is_empty());
    assert_eq!(host.active_work(), 0);
}
fn config() -> DriverConfig {
    DriverConfig {
        profile: sandproof_controller_service::CheckpointProfile::CreateKill,
        node: local::wire::node("host-a"),
        compute: local::compute(),
        interval: Duration::from_millis(100),
    }
}

#[tokio::test]
async fn durable_scan_advances_original_create_then_destroy_without_manual_request_queue() {
    let dir = tempfile::tempdir().unwrap();
    let state = Arc::new(Mutex::new(local::State::default()));
    let app = local::application(
        &dir.path().join("db"),
        Arc::clone(&state),
        Arc::new(local::TestClock(AtomicI64::new(10))),
        true,
    );
    let host = HostLifecycle::new(app);
    let (observed_at_ms, free) = host
        .free_resources(config().node.clone(), config().compute)
        .await
        .unwrap();
    // Exactly the production StatusReceiver projection, sourced from the
    // original application and shared non-waiting worker gate, not audit rows.
    let status = HostStatus {
        node: config().node,
        retention_slots: 2,
        observed_at_ms,
        free,
    };
    status
        .validate_for(&StatusRequest {
            node: config().node,
        })
        .unwrap();
    assert_eq!(status.observed_at_ms, 10);
    assert_eq!(status.free.instances, 2);
    assert_eq!(
        serde_json::from_slice::<HostStatus>(&serde_json::to_vec(&status).unwrap()).unwrap(),
        status
    );
    let create = local::request();
    host.receive(create.clone()).await.unwrap();
    assert_eq!(
        host.free_resources(config().node, config().compute)
            .await
            .unwrap()
            .1
            .instances,
        1
    );
    assert!(state.lock().unwrap().actions.is_empty());
    let report = host
        .drive_once(config(), Arc::new(Templates))
        .await
        .unwrap();
    assert_eq!(
        (report.scanned, report.advanced, report.unavailable),
        (1, 1, 0)
    );
    assert!(matches!(
        host.receive(create.clone()).await.unwrap().fact,
        LifecycleFact::Ready { .. }
    ));
    assert_eq!(
        host.free_resources(config().node, config().compute)
            .await
            .unwrap()
            .1
            .instances,
        1
    );
    let launches = state.lock().unwrap().actions.len();
    let idle = host.drive_once(config(), Arc::new(Missing)).await.unwrap();
    assert_eq!(idle, sandproof_host_service::DriveReport::default());
    assert_eq!(state.lock().unwrap().actions.len(), launches);
    let mut destroy = create;
    destroy.intent = Intent::Destroy;
    destroy.identity.operation_id = "operation-2".try_into().unwrap();
    destroy.identity.intent_version = 2.try_into().unwrap();
    host.receive(destroy.clone()).await.unwrap();
    host.drive_once(config(), Arc::new(Missing)).await.unwrap();
    let cleaned = host.receive(destroy).await.unwrap();
    assert_eq!(cleaned.fact, LifecycleFact::Cleaned);
    assert_eq!(
        host.free_resources(config().node, config().compute)
            .await
            .unwrap()
            .1
            .instances,
        1
    );
    host.confirm(cleaned.clone()).await.unwrap();
    host.close(sandproof_protocol::internal::window::WindowCloseRequest::new(cleaned))
        .await
        .unwrap();
    assert_eq!(
        host.free_resources(config().node, config().compute)
            .await
            .unwrap()
            .1
            .instances,
        2
    );
    assert_eq!(host.active_work(), 0);
}

#[tokio::test]
async fn missing_material_does_not_start_vm_or_suppress_another_durable_request() {
    let dir = tempfile::tempdir().unwrap();
    let state = Arc::new(Mutex::new(local::State::default()));
    let app = local::application(
        &dir.path().join("db"),
        Arc::clone(&state),
        Arc::new(local::TestClock(AtomicI64::new(10))),
        true,
    );
    let host = HostLifecycle::new(app);
    let mut missing = local::request();
    missing.create.template_revision = "absent-revision".try_into().unwrap();
    host.receive(missing.clone()).await.unwrap();
    let mut healthy = local::request();
    healthy.identity.operation_id = "operation-2".try_into().unwrap();
    healthy.identity.sandbox_id = "sandbox-2".try_into().unwrap();
    healthy.identity.retention_slot = 2;
    host.receive(healthy.clone()).await.unwrap();
    let report = host
        .drive_once(config(), Arc::new(Templates))
        .await
        .unwrap();
    assert_eq!(
        (report.scanned, report.advanced, report.unavailable),
        (2, 1, 1)
    );
    assert_eq!(
        host.receive(missing).await.unwrap().fact,
        LifecycleFact::Recorded
    );
    assert!(matches!(
        host.receive(healthy).await.unwrap().fact,
        LifecycleFact::Ready { .. }
    ));
}

#[tokio::test]
async fn disconnected_center_and_unavailable_template_cannot_prevent_local_expiry() {
    let dir = tempfile::tempdir().unwrap();
    let state = Arc::new(Mutex::new(local::State::default()));
    let clock = Arc::new(local::TestClock(AtomicI64::new(10)));
    let app = local::application(
        &dir.path().join("db"),
        Arc::clone(&state),
        Arc::clone(&clock),
        true,
    );
    let host = HostLifecycle::new(Arc::clone(&app));
    let create = local::request();
    host.receive(create).await.unwrap();
    host.drive_once(config(), Arc::new(Templates))
        .await
        .unwrap();
    let token = state
        .lock()
        .unwrap()
        .configurations
        .values()
        .next()
        .unwrap()
        .0
        .clone();
    assert!(
        app.authorize_data(
            &token,
            sandproof_controller_service::application::DataKind::Exec
        )
        .is_ok()
    );
    clock.0.store(60_010, Ordering::SeqCst);
    host.drive_once(config(), Arc::new(Missing)).await.unwrap();
    assert_eq!(
        app.authorize_data(
            &token,
            sandproof_controller_service::application::DataKind::Exec
        ),
        Err(ApplicationError::NotFound)
    );
    assert_eq!(host.active_work(), 0);
}

#[tokio::test]
async fn explicit_shutdown_never_implicitly_creates_or_destroys_instances() {
    let dir = tempfile::tempdir().unwrap();
    let state = Arc::new(Mutex::new(local::State::default()));
    let host = HostLifecycle::new(local::application(
        &dir.path().join("db"),
        Arc::clone(&state),
        Arc::new(local::TestClock(AtomicI64::new(10))),
        true,
    ));
    let (_signal, shutdown) = tokio::sync::watch::channel(true);
    host.run_driver(config(), Arc::new(Templates), shutdown)
        .await
        .unwrap();
    assert!(state.lock().unwrap().actions.is_empty());
    let mut invalid = config();
    invalid.interval = Duration::ZERO;
    assert_eq!(
        host.drive_once(invalid, Arc::new(Templates)).await,
        Err(ApplicationError::Invalid)
    );
    assert_eq!(host.active_work(), 0);
}
