//! Real original SQLite/application; controlled physical effects, not KVM.
#[allow(dead_code, reason = "shared original application fixture")]
#[path = "../../controller-service/tests/support/internal_application.rs"]
mod local;
use sandproof_controller_service::{InternalTemplate, application::ApplicationError};
use sandproof_host_service::{DriverConfig, HostLifecycle, TemplateSource};
use sandproof_internal::LifecycleReceiver;
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
    let create = local::request();
    host.receive(create.clone()).await.unwrap();
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
    assert_eq!(
        host.receive(destroy).await.unwrap().fact,
        LifecycleFact::Cleaned
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
