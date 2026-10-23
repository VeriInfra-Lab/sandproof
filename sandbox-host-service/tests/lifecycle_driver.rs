//! Original v3 application and `SQLite`; host effects/epoch are controlled.
#[allow(dead_code, reason = "shared original application fixture")]
#[path = "../../controller-service/tests/support/lifecycle_application.rs"]
mod local;

use sandproof_controller_service::{
    CheckpointProfile, CreateReadiness, InternalTemplate, application::ApplicationError,
};
use sandproof_host_service::{DriveReport, DriverConfig, HostLifecycle, TemplateSource};
use sandproof_internal::LifecycleReceiver;
use sandproof_protocol::{
    internal::lifecycle::CreateBinding,
    lifecycle::{AcceptedLifecycleIntent, LifecycleOperation},
};
use std::{
    sync::{Arc, Mutex},
    time::Duration,
};

struct Templates;
impl TemplateSource for Templates {
    fn bound(&self, binding: &CreateBinding) -> Result<InternalTemplate, ApplicationError> {
        let original = local::fixture::template();
        if binding.template_revision != original.revision
            || binding.template_digest != original.digest
        {
            return Err(ApplicationError::NotFound);
        }
        Ok(original)
    }
}
fn config() -> DriverConfig {
    DriverConfig {
        profile: CheckpointProfile::FirstUnload,
        node: local::config().node,
        compute: local::fixture::compute(),
        interval: Duration::from_millis(100),
    }
}
fn matching_host() -> Arc<Mutex<local::fixture::State>> {
    Arc::new(Mutex::new(local::fixture::State {
        epoch: local::fixture::TestEpoch::Matching,
        source: Some(
            sandproof_controller_service::capture::SourceProcessIdentity {
                pid: 1234,
                start_time_ticks: 5678,
                boot_id: "fixture-boot".into(),
            },
        ),
        ..local::fixture::State::default()
    }))
}

#[tokio::test]
async fn same_original_driver_starts_recorded_create_then_restart_never_recreates_it() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("db");
    let physical = matching_host();
    let app = Arc::new(local::application(&path, physical.clone(), true));
    app.establish_lifecycle_host_epoch().unwrap();
    let host = HostLifecycle::new(app.clone());
    let request = local::request();
    host.record_v3(request.clone(), local::context())
        .await
        .unwrap();
    assert!(physical.lock().unwrap().actions.is_empty());
    assert_eq!(
        host.drive_once(config(), Arc::new(Templates))
            .await
            .unwrap(),
        DriveReport {
            scanned: 1,
            advanced: 1,
            unavailable: 0,
        }
    );
    let original = app
        .lifecycle_work_state(&request, &local::context())
        .unwrap();
    assert!(original.local.is_some());
    assert!(matches!(
        app.observe_lifecycle_create(&request, &local::context())
            .unwrap()
            .readiness,
        CreateReadiness::Observed(_)
    ));
    let calls = physical.lock().unwrap().actions.len();
    assert!(calls > 0);
    assert_eq!(
        host.drive_once(config(), Arc::new(Templates))
            .await
            .unwrap(),
        DriveReport::default()
    );
    assert_eq!(physical.lock().unwrap().actions.len(), calls);
    drop((host, app));
    let app = Arc::new(local::application(&path, physical.clone(), false));
    // The reopened manager checks its original registered requirement again;
    // this fixture supplies controlled matching host facts, not a real payload.
    app.establish_lifecycle_host_epoch().unwrap();
    let host = HostLifecycle::new(app.clone());
    assert_eq!(
        host.drive_once(config(), Arc::new(Templates))
            .await
            .unwrap(),
        DriveReport::default()
    );
    assert_eq!(
        app.lifecycle_work_state(&request, &local::context())
            .unwrap(),
        original
    );
    assert!(matches!(
        app.observe_lifecycle_create(&request, &local::context())
            .unwrap()
            .readiness,
        CreateReadiness::Observed(_)
    ));
    assert_eq!(physical.lock().unwrap().actions.len(), calls);
    assert_eq!(host.active_work(), 0);
}

#[tokio::test]
async fn first_missing_material_keeps_responsibility_and_does_not_starve_next_create() {
    let dir = tempfile::tempdir().unwrap();
    let physical = matching_host();
    let app = Arc::new(local::application(&dir.path().join("db"), physical, true));
    app.establish_lifecycle_host_epoch().unwrap();
    let host = HostLifecycle::new(app.clone());
    let mut missing = local::request();
    let LifecycleOperation::Create { create, .. } = &mut missing.request.operation else {
        unreachable!()
    };
    create.template_revision = "missing".try_into().unwrap();
    host.record_v3(missing.clone(), local::context())
        .await
        .unwrap();
    let mut healthy = local::request();
    healthy.request.binding.session = "second-session".try_into().unwrap();
    healthy.request.binding.key.sequence = 2.try_into().unwrap();
    host.record_v3(healthy.clone(), local::context())
        .await
        .unwrap();
    assert_eq!(
        host.drive_once(config(), Arc::new(Templates))
            .await
            .unwrap(),
        DriveReport {
            scanned: 2,
            advanced: 1,
            unavailable: 1,
        }
    );
    let pending = app
        .lifecycle_work_state(&missing, &local::context())
        .unwrap();
    assert!(pending.local.is_none());
    assert!(pending.current);
    assert!(!pending.terminal);
    assert_eq!(
        app.lifecycle_pending_creates(&config().node).unwrap(),
        vec![missing]
    );
    assert!(matches!(
        app.observe_lifecycle_create(&healthy, &local::context())
            .unwrap()
            .readiness,
        CreateReadiness::Observed(_)
    ));
}

#[tokio::test]
async fn missing_epoch_cannot_start_or_turn_original_accepted_work_into_not_accepted() {
    let dir = tempfile::tempdir().unwrap();
    let physical = matching_host();
    let app = Arc::new(local::application(
        &dir.path().join("db"),
        physical.clone(),
        true,
    ));
    let host = HostLifecycle::new(app.clone());
    let request = local::request();
    host.record_v3(request.clone(), local::context())
        .await
        .unwrap();
    assert_eq!(
        host.drive_once(config(), Arc::new(Templates))
            .await
            .unwrap(),
        DriveReport {
            scanned: 1,
            advanced: 0,
            unavailable: 1,
        }
    );
    let state = app
        .lifecycle_work_state(&request, &local::context())
        .unwrap();
    assert!(state.current && !state.terminal && state.local.is_none());
    assert_eq!(
        app.lifecycle_pending_creates(&config().node).unwrap(),
        vec![request]
    );
    assert!(physical.lock().unwrap().actions.is_empty());
}

#[tokio::test]
async fn wrong_driver_profile_refuses_instead_of_falling_back_to_legacy_create() {
    let dir = tempfile::tempdir().unwrap();
    let physical = matching_host();
    let app = Arc::new(local::application(
        &dir.path().join("db"),
        physical.clone(),
        true,
    ));
    app.establish_lifecycle_host_epoch().unwrap();
    let host = HostLifecycle::new(app.clone());
    let request: AcceptedLifecycleIntent = local::request();
    host.record_v3(request.clone(), local::context())
        .await
        .unwrap();
    let mut wrong = config();
    wrong.profile = CheckpointProfile::CreateKill;
    assert_eq!(
        host.drive_once(wrong, Arc::new(Templates)).await,
        Err(ApplicationError::OutcomeUnknown)
    );
    assert_eq!(
        app.lifecycle_pending_creates(&config().node).unwrap(),
        vec![request]
    );
    assert!(physical.lock().unwrap().actions.is_empty());
}

#[tokio::test]
async fn native_running_without_original_source_stays_historical_and_not_current_ready() {
    let dir = tempfile::tempdir().unwrap();
    let physical = matching_host();
    physical.lock().unwrap().source = None;
    let app = Arc::new(local::application(&dir.path().join("db"), physical, true));
    app.establish_lifecycle_host_epoch().unwrap();
    let host = HostLifecycle::new(app.clone());
    let request = local::request();
    host.record_v3(request.clone(), local::context())
        .await
        .unwrap();
    assert_eq!(
        host.drive_once(config(), Arc::new(Templates))
            .await
            .unwrap(),
        DriveReport {
            scanned: 1,
            advanced: 1,
            unavailable: 0,
        }
    );
    let view = app
        .observe_lifecycle_create(&request, &local::context())
        .unwrap();
    assert_eq!(
        view.historical_result,
        Some(sandbox_controller_core::OperationResult::Running(1))
    );
    assert_eq!(
        view.readiness,
        CreateReadiness::Pending(
            sandproof_controller_service::CreateReadinessPending::OriginalEvidenceMissing,
        )
    );
}
