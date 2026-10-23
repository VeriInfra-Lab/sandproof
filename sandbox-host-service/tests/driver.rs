//! Current durable inbox/application and real Tokio tasks; host facts are
//! controlled component inputs. This is not a KVM or whole-system result.
#[allow(dead_code, reason = "shared original application fixture")]
#[path = "../../controller-service/tests/support/lifecycle_application.rs"]
mod current;
use current::fixture as local;
use sandproof_controller_service::{
    CreateReadiness, InternalTemplate,
    application::{ApplicationError, ControllerApplication},
    capture::SourceProcessIdentity,
};
use sandproof_host_service::{DriverConfig, HostLifecycle, TemplateSource};
use sandproof_internal::{
    LifecycleReceiver,
    status::{HostStatus, StatusRequest},
};
use sandproof_protocol::{
    internal::lifecycle::CreateBinding,
    lifecycle::{
        AcceptedLifecycleIntent, ExpectedSession, LifecycleOperation, LifecycleWindowClose,
        TerminalStatus,
    },
};
use std::{
    path::Path,
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
fn state() -> Arc<Mutex<local::State>> {
    Arc::new(Mutex::new(local::State {
        epoch: local::TestEpoch::Matching,
        source: Some(SourceProcessIdentity {
            pid: 1234,
            start_time_ticks: 5678,
            boot_id: "fixture-boot".into(),
        }),
        ..local::State::default()
    }))
}
fn application(
    path: &Path,
    state: Arc<Mutex<local::State>>,
    clock: Arc<local::TestClock>,
) -> Arc<ControllerApplication<local::TestHost>> {
    let app = Arc::new(current::application(path, state, true).with_clock(clock));
    app.establish_lifecycle_host_epoch().unwrap();
    app
}
fn config() -> DriverConfig {
    DriverConfig {
        node: current::config().node,
        compute: local::compute(),
        interval: Duration::from_millis(100),
    }
}
fn second(mut request: AcceptedLifecycleIntent) -> AcceptedLifecycleIntent {
    request.request.binding.session = "session-b".try_into().unwrap();
    request.request.binding.key.sequence = 2.try_into().unwrap();
    request.retention.slot = 2;
    request
}
async fn work_drained(host: &HostLifecycle<local::TestHost>) {
    tokio::time::timeout(Duration::from_secs(5), async {
        while host.active_work() != 0 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
}
struct Release(Option<std::sync::mpsc::Sender<()>>);
impl Drop for Release {
    fn drop(&mut self) {
        if let Some(sender) = self.0.take() {
            let _ = sender.send(());
        }
    }
}

#[tokio::test]
async fn health_progresses_during_blocked_work_and_reads_original_committed_usage() {
    let dir = tempfile::tempdir().unwrap();
    let (entered, observed) = std::sync::mpsc::channel();
    let (release, blocked) = std::sync::mpsc::channel();
    let mut release = Release(Some(release));
    let state = state();
    state.lock().unwrap().block_prepare = Some((entered, blocked));
    let app = application(
        &dir.path().join("db"),
        state,
        Arc::new(local::TestClock(AtomicI64::new(10))),
    );
    let host = HostLifecycle::new(app);
    let create = current::request();
    host.record_lifecycle(create.clone(), current::context())
        .await
        .unwrap();
    let original = host.clone();
    let work = tokio::spawn(async move {
        original
            .advance(create, Some(local::template()), local::compute())
            .await
    });
    tokio::task::spawn_blocking(move || observed.recv_timeout(Duration::from_secs(5)).unwrap())
        .await
        .unwrap();
    let (at, free) = tokio::time::timeout(
        Duration::from_secs(5),
        host.free_resources(config().node, config().compute),
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(at, 10);
    assert_eq!(free.instances, 1);
    assert_eq!(
        host.active_work(),
        1,
        "sampling must finish while actual Prepare remains held"
    );
    release.0.take().unwrap().send(()).unwrap();
    assert!(work.await.unwrap().unwrap().local.is_some());
    assert_eq!(host.active_work(), 0);
}

struct CorruptAuthority(std::path::PathBuf);
impl TemplateSource for CorruptAuthority {
    fn bound(&self, _: &CreateBinding) -> Result<InternalTemplate, ApplicationError> {
        // Foreign SQLite writes are a deterministic test fault, never a second
        // production writer. Both candidates must recheck before admission.
        rusqlite::Connection::open(&self.0)
            .unwrap()
            .execute(
                "UPDATE controller_lifecycle_inbox SET sha256=zeroblob(32) WHERE id=1",
                [],
            )
            .unwrap();
        Ok(local::template())
    }
}
#[tokio::test]
async fn authority_fault_during_advance_halts_before_another_healthy_request() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("db");
    let state = state();
    let app = application(
        &path,
        state.clone(),
        Arc::new(local::TestClock(AtomicI64::new(10))),
    );
    let host = HostLifecycle::new(app);
    for request in [current::request(), second(current::request())] {
        host.record_lifecycle(request, current::context())
            .await
            .unwrap();
    }
    assert_eq!(
        host.drive_once(config(), Arc::new(CorruptAuthority(path)))
            .await,
        Err(ApplicationError::OutcomeUnknown)
    );
    work_drained(&host).await;
    assert!(state.lock().unwrap().actions.is_empty());
    assert_eq!(host.active_work(), 0);
}

#[tokio::test]
async fn durable_scan_advances_original_create_then_destroy_without_manual_request_queue() {
    let dir = tempfile::tempdir().unwrap();
    let state = state();
    let app = application(
        &dir.path().join("db"),
        state.clone(),
        Arc::new(local::TestClock(AtomicI64::new(10))),
    );
    let host = HostLifecycle::new(app.clone());
    let (observed_at_ms, free) = host
        .free_resources(config().node, config().compute)
        .await
        .unwrap();
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
    let create = current::request();
    host.record_lifecycle(create.clone(), current::context())
        .await
        .unwrap();
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
        app.observe_lifecycle_create(&create, &current::context())
            .unwrap()
            .readiness,
        CreateReadiness::Observed(_)
    ));
    let launches = state.lock().unwrap().actions.len();
    assert_eq!(
        host.drive_once(config(), Arc::new(Missing)).await.unwrap(),
        sandproof_host_service::DriveReport::default()
    );
    assert_eq!(state.lock().unwrap().actions.len(), launches);
    let mut destroy = create;
    destroy.accepted_version = 2;
    destroy.request.binding.expected = ExpectedSession::Version(1);
    destroy.request.binding.key.sequence = 2.try_into().unwrap();
    destroy.request.operation = LifecycleOperation::Destroy {};
    host.record_lifecycle(destroy.clone(), current::context())
        .await
        .unwrap();
    host.drive_once(config(), Arc::new(Missing)).await.unwrap();
    let cleaned = host
        .observe_terminal(destroy, current::context())
        .await
        .unwrap();
    assert_eq!(cleaned.status, TerminalStatus::Cleaned);
    assert_eq!(
        host.free_resources(config().node, config().compute)
            .await
            .unwrap()
            .1
            .instances,
        1
    );
    assert!(
        host.confirm_terminal(cleaned.clone(), current::context())
            .await
            .unwrap()
            .matches(&cleaned)
    );
    let close = LifecycleWindowClose { terminal: cleaned };
    assert!(
        host.close_window(close.clone(), current::context())
            .await
            .unwrap()
            .matches(&close)
    );
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
    let state = state();
    let app = application(
        &dir.path().join("db"),
        state,
        Arc::new(local::TestClock(AtomicI64::new(10))),
    );
    let host = HostLifecycle::new(app.clone());
    let mut missing = current::request();
    let LifecycleOperation::Create { create, .. } = &mut missing.request.operation else {
        panic!()
    };
    create.template_revision = "absent-revision".try_into().unwrap();
    host.record_lifecycle(missing.clone(), current::context())
        .await
        .unwrap();
    let healthy = second(current::request());
    host.record_lifecycle(healthy.clone(), current::context())
        .await
        .unwrap();
    let report = host
        .drive_once(config(), Arc::new(Templates))
        .await
        .unwrap();
    assert_eq!(
        (report.scanned, report.advanced, report.unavailable),
        (2, 1, 1)
    );
    let pending = app
        .lifecycle_work_state(&missing, &current::context())
        .unwrap();
    assert!(pending.current && !pending.terminal && pending.local.is_none());
    assert!(matches!(
        app.observe_lifecycle_create(&healthy, &current::context())
            .unwrap()
            .readiness,
        CreateReadiness::Observed(_)
    ));
}

#[tokio::test]
async fn disconnected_center_and_unavailable_template_cannot_prevent_local_expiry() {
    let dir = tempfile::tempdir().unwrap();
    let state = state();
    let clock = Arc::new(local::TestClock(AtomicI64::new(10)));
    let app = application(&dir.path().join("db"), state.clone(), clock.clone());
    let host = HostLifecycle::new(app.clone());
    host.record_lifecycle(current::request(), current::context())
        .await
        .unwrap();
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
    let state = state();
    let host = HostLifecycle::new(application(
        &dir.path().join("db"),
        state.clone(),
        Arc::new(local::TestClock(AtomicI64::new(10))),
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
